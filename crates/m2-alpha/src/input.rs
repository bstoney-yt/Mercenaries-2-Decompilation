//! Actions, their key bindings, and the input state the game reads each
//! tick.

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
    /// Turn an overturned vehicle back up.
    Reset,
    /// Fire the weapon in hand (also the left mouse button).
    Fire,
    /// Raise the weapon (also the right mouse button, once the mouse is
    /// captured).
    Aim,
    /// Throw a grenade.
    Throw,
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
    Quit,
    /// The pad's face buttons Y, A, X and B, which a hijack's prompts ask
    /// for ([`crate::hijack`]); the left stick's pushes are the movement
    /// keys.
    PadY,
    PadA,
    PadX,
    PadB,
}

/// Which keys trigger each action, as written in the settings file
/// (`move_forward = ["KeyW", "ArrowUp"]`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bindings(pub BTreeMap<Action, Vec<KeyCode>>);

impl Default for Bindings {
    fn default() -> Self {
        use Action::*;
        use KeyCode::*;
        Self(BTreeMap::from([
            (MoveForward, vec![KeyW, ArrowUp]),
            (MoveBack, vec![KeyS, ArrowDown]),
            (MoveLeft, vec![KeyA, ArrowLeft]),
            (MoveRight, vec![KeyD, ArrowRight]),
            (MoveUp, vec![Space]),
            (MoveDown, vec![KeyC, ControlLeft]),
            (Sprint, vec![ShiftLeft, ShiftRight]),
            (Walk, vec![KeyQ]),
            (ToggleFly, vec![KeyV]),
            (ToggleMusic, vec![KeyM]),
            (Use, vec![KeyE]),
            (Reset, vec![KeyR]),
            (Fire, vec![KeyF]),
            (Aim, vec![]),
            (Throw, vec![KeyG]),
            (Melee, vec![KeyB]),
            (Rifle, vec![Digit1]),
            (Launcher, vec![Digit2]),
            (Support, vec![KeyT]),
            (Pda, vec![KeyP]),
            (ToggleMouse, vec![Tab]),
            (Fullscreen, vec![F11]),
            (NextGait, vec![KeyN]),
            (ToggleCulling, vec![F3]),
            (QuickSave, vec![F5]),
            (QuickLoad, vec![F9]),
            (Quit, vec![Escape]),
            // Laid out as the face buttons are (not the PC game's keys,
            // which aren't known yet).
            (PadY, vec![KeyI]),
            (PadA, vec![KeyK]),
            (PadX, vec![KeyJ]),
            (PadB, vec![KeyL]),
        ]))
    }
}

impl Bindings {
    /// The actions `key` triggers.
    pub fn actions_for(&self, key: KeyCode) -> impl Iterator<Item = Action> + '_ {
        self.0
            .iter()
            .filter(move |(_, keys)| keys.contains(&key))
            .map(|(&a, _)| a)
    }

    /// The name of the first key bound to `action` (`P`), if any.
    pub fn key_name(&self, action: Action) -> Option<String> {
        self.0.get(&action)?.first().map(|&k| key_name(k))
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

/// A line of the game's text with its key tokens (`[pda] Open PDA`) named
/// as the keys bound here, in brackets (`[P] Open PDA`): `[pda]` the PDA's
/// key, `[support]` the support menu's, `[pfire]` the PC's fire (the left
/// mouse button), `[confirm]` the consoles' (here the use key), `[move]`
/// the keys that move, `[sprint]` sprint's (round 98 met it after the
/// first gate). Other tokens, and keys left unbound, stay as they are.
pub fn name_keys(line: &str, keys: &Bindings) -> String {
    use Action::*;
    let name = |a| keys.key_name(a);
    let named = |token: &str| match token {
        "pda" => name(Pda),
        "support" => name(Support),
        "confirm" => name(Use),
        "sprint" => name(Sprint),
        "pfire" => Some("Left click".to_owned()),
        "move" => {
            let names: Vec<String> = [MoveForward, MoveLeft, MoveBack, MoveRight]
                .into_iter()
                .filter_map(name)
                .collect();
            let letters = names.iter().all(|n| n.chars().count() == 1);
            (!names.is_empty()).then(|| names.join(if letters { "" } else { "/" }))
        }
        _ => None,
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

/// Held actions, actions pressed since the last tick, and mouse motion.
#[derive(Debug, Default)]
pub struct Input {
    held: HashSet<Action>,
    pressed: HashSet<Action>,
    mouse: (f64, f64),
}

impl Input {
    pub fn key(&mut self, bindings: &Bindings, key: KeyCode, down: bool, repeat: bool) {
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

    /// Releases everything (the window lost focus).
    pub fn clear(&mut self) {
        self.held.clear();
        self.pressed.clear();
        self.mouse = (0.0, 0.0);
    }

    pub fn held(&self, action: Action) -> bool {
        self.held.contains(&action)
    }

    /// True once per key press.
    pub fn take_pressed(&mut self, action: Action) -> bool {
        self.pressed.remove(&action)
    }

    /// Mouse movement since the last call.
    pub fn take_mouse(&mut self) -> (f64, f64) {
        std::mem::take(&mut self.mouse)
    }

    /// -1, 0 or 1 from a pair of opposing actions.
    pub fn axis(&self, negative: Action, positive: Action) -> f32 {
        f32::from(i8::from(self.held(positive)) - i8::from(self.held(negative)))
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
        assert_eq!(name_keys("[pda] Look", &b), "[P] Look");
        assert_eq!(
            name_keys("[support] One\n[pfire] Two [move] [confirm]", &b),
            "[T] One\n[Left click] Two [WASD] [E]"
        );
        assert_eq!(name_keys("[sprint] Sprint", &b), "[Shift] Sprint");
        assert_eq!(name_keys("[other] stays [", &b), "[other] stays [");
        let mut b = b;
        b.0.insert(Action::MoveForward, vec![KeyCode::ArrowUp]);
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
        let f: File =
            toml::from_str("[keys]\nmove_forward = [\"KeyZ\"]\nquit = [\"KeyQ\"]").unwrap();
        assert_eq!(
            f.keys.actions_for(KeyCode::KeyZ).collect::<Vec<_>>(),
            [Action::MoveForward]
        );
        assert_eq!(
            f.keys.actions_for(KeyCode::KeyQ).collect::<Vec<_>>(),
            [Action::Quit]
        );
    }
}
