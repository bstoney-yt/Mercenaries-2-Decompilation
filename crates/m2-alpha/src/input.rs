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
    /// Capture or release the mouse for looking around.
    ToggleMouse,
    Fullscreen,
    /// Step the shown characters through idle, walk, run and sprint.
    NextGait,
    /// Draw back faces too (a debugging aid while formats are checked).
    ToggleCulling,
    Quit,
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
            (ToggleMouse, vec![Tab]),
            (Fullscreen, vec![F11]),
            (NextGait, vec![KeyN]),
            (ToggleCulling, vec![F3]),
            (Quit, vec![Escape]),
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
