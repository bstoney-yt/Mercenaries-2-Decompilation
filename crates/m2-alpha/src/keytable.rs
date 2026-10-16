//! The pause menu's keyboard and mouse table (Options, Controls,
//! Keyboard/Mouse): its 24 actions, each with a primary and a secondary
//! key, as the PC game lists them, and the alpha's bindings behind each.
//!
//! The movie (`pause_menu`) keeps the table in `LTIInputKMTable`: column 0
//! the actions' names, 1 the primary keys, 2 the secondary ones. The
//! engine fills it (`LTIInputKMSetActionName(row, name)`,
//! `LTIInputKMKeyMap(row, secondary, key)`) and has it drawn again
//! (`LTIInputKMUpdateTable`). The movie counts the rows (`lines`, which
//! its scrolling stops at) as soon as it has said `LTIInputKMEnter`, so
//! the table is filled before then ([`crate::pause`]). Picking a cell, the
//! movie says `LTIInputKMChangeInput` with the row plus 100 for the
//! secondary column and shows `???` until the engine has the next key
//! (`LTIFinishedGettingInput`); then Accept (`LTIInputKMApplyChanges`),
//! Cancel and Defaults (`LTIInputKMDefault`).
//!
//! # Provenance
//!
//! From the game's data: the rows' names are the English strings'
//! `SHELL.PCShell.*` texts (ids solved by their hash: `MoveForward`,
//! `PrimaryAttack2`...; three whose ids are not solved are named by key);
//! empty cells say `SHELL.PCShell.EMPTY`; the movie's calls above.
//! From the PC footage (the Controls screen, 1:46-2:04): the rows' order,
//! the default keys and how keys are written (`W`, `MOUSE1`, `MWHEEL`,
//! `SPACE`, `LCTRL`, `UP`). The names of keys the footage does not show
//! (`RCTRL`, `ENTER`...) follow the same pattern (invented). Which of the
//! alpha's actions each row moves is inferred from the names. A key
//! already in another cell moves to the new one, leaving that cell empty
//! (invented: the game puts up a "Key Over Bound" box, not done here).

use winit::keyboard::KeyCode;

use crate::input::{Action, Bindings, Control, Mouse};

/// What an empty cell says: `SHELL.PCShell.EMPTY` ("EMPTY").
pub const EMPTY: &str = "[SHELL.PCShell.EMPTY]";

/// A row of the table: the text id of its name and the alpha's actions
/// its keys trigger (the first one's keys are the row's).
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub name: &'static str,
    pub actions: &'static [Action],
}

/// The table's rows, in the footage's order.
pub const ROWS: [Row; 24] = {
    use Action::*;
    const fn row(name: &'static str, actions: &'static [Action]) -> Row {
        Row { name, actions }
    }
    [
        row("[SHELL.PCShell.MoveForward]", &[MoveForward]),
        row("[SHELL.PCShell.MoveBackward]", &[MoveBack]),
        row("[SHELL.PCShell.StrafeLeft]", &[MoveLeft]),
        row("[SHELL.PCShell.StrafeRight]", &[MoveRight]),
        row("[SHELL.PCShell.PrimaryAttack2]", &[Fire]),
        row("[SHELL.PCShell.PrimarySwitch2]", &[SwitchWeapon]),
        row("[SHELL.PCShell.Reload2]", &[Reload, Reset]),
        row("[SHELL.PCShell.SecondaryAttack2]", &[Throw]),
        row("[SHELL.PCShell.SecondarySwitch2]", &[Explosive]),
        row("[SHELL.PCShell.MeleeAttack2]", &[Melee]),
        row("[SHELL.PCShell.Use2]", &[Use]),
        row("[SHELL.PCShell.Jump2]", &[MoveUp]),
        row("[SHELL.PCShell.Crouch2]", &[MoveDown, Aim]),
        row("[SHELL.PCShell.Binoculars2]", &[Binoculars]),
        row("[SHELL.PCShell.PDA2]", &[Pda]),
        row("[SHELL.PCShell.LookUp2]", &[LookUp]),
        row("[SHELL.PCShell.LookDown2]", &[LookDown]),
        row("[SHELL.PCShell.LookLeft2]", &[LookLeft]),
        row("[SHELL.PCShell.LookRight2]", &[LookRight]),
        // "Support Up", "Support Down", "View Switch": ids not solved.
        row("[0xf98f60b1]", &[Support]),
        row("[0xd7c932d9]", &[SupportDown]),
        row("[0x768777b1]", &[ViewSwitch]),
        row("[SHELL.PCShell.Walk3]", &[Walk]),
        row("[SHELL.Controls.Sprint]", &[Sprint]),
    ]
};

/// What a cell holds: a key or mouse button, or the wheel (both its
/// notches, as the game binds it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Control(Control),
    Wheel,
}

impl From<Control> for Slot {
    fn from(c: Control) -> Self {
        match c {
            Control::Mouse(Mouse::WheelUp | Mouse::WheelDown) => Self::Wheel,
            c => Self::Control(c),
        }
    }
}

impl Slot {
    /// The controls the cell binds.
    fn controls(self) -> Vec<Control> {
        match self {
            Self::Control(c) => vec![c],
            Self::Wheel => vec![Mouse::WheelUp.into(), Mouse::WheelDown.into()],
        }
    }

    /// The cell's text: `W`, `MOUSE1`, `MWHEEL`, `LCTRL`.
    pub fn name(self) -> String {
        use KeyCode::*;
        let key = match self {
            Self::Wheel => return "MWHEEL".into(),
            Self::Control(Control::Mouse(m)) => {
                return match m {
                    Mouse::Left => "MOUSE1",
                    Mouse::Right => "MOUSE2",
                    Mouse::Middle => "MOUSE3",
                    Mouse::WheelUp | Mouse::WheelDown => "MWHEEL",
                }
                .into();
            }
            Self::Control(Control::Key(k)) => k,
        };
        let debug = format!("{key:?}");
        if let Some(short) = debug
            .strip_prefix("Key")
            .or_else(|| debug.strip_prefix("Digit"))
        {
            return short.to_owned();
        }
        match key {
            Space => "SPACE",
            Tab => "TAB",
            ControlLeft => "LCTRL",
            ControlRight => "RCTRL",
            ShiftLeft => "LSHIFT",
            ShiftRight => "RSHIFT",
            AltLeft => "LALT",
            AltRight => "RALT",
            ArrowUp => "UP",
            ArrowDown => "DOWN",
            ArrowLeft => "LEFT",
            ArrowRight => "RIGHT",
            Escape => "ESC",
            _ => return debug.to_uppercase(),
        }
        .into()
    }
}

/// The table: each row's primary and secondary cell.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyTable(pub [[Option<Slot>; 2]; ROWS.len()]);

impl Default for KeyTable {
    /// The game's defaults (the alpha's default bindings).
    fn default() -> Self {
        Self::of(&Bindings::default())
    }
}

impl KeyTable {
    /// The table as `bindings` bind: each row's first action's first two
    /// controls (the wheel's notches as one).
    pub fn of(bindings: &Bindings) -> Self {
        let mut cells = [[None; 2]; ROWS.len()];
        for (row, cell) in ROWS.iter().zip(&mut cells) {
            let mut slots: Vec<Slot> = Vec::new();
            for &c in bindings.0.get(&row.actions[0]).into_iter().flatten() {
                let s = Slot::from(c);
                if !slots.contains(&s) {
                    slots.push(s);
                }
            }
            for (i, s) in slots.into_iter().take(2).enumerate() {
                cell[i] = Some(s);
            }
        }
        Self(cells)
    }

    /// Binds each row's actions to its cells' controls, as the table
    /// says (other actions keep theirs).
    pub fn apply(&self, bindings: &mut Bindings) {
        for (row, cell) in ROWS.iter().zip(&self.0) {
            let controls: Vec<Control> = cell.iter().flatten().flat_map(|s| s.controls()).collect();
            for &a in row.actions {
                bindings.0.insert(a, controls.clone());
            }
        }
    }

    /// Puts `slot` in `row`'s cell `column` (0 primary, 1 secondary),
    /// taking it out of any other cell: the rows changed.
    pub fn set(&mut self, row: usize, column: usize, slot: Slot) -> Vec<usize> {
        let mut changed = vec![row];
        for (r, cell) in self.0.iter_mut().enumerate() {
            for (c, s) in cell.iter_mut().enumerate() {
                if *s == Some(slot) && (r, c) != (row, column) {
                    *s = None;
                    if !changed.contains(&r) {
                        changed.push(r);
                    }
                }
            }
        }
        self.0[row][column] = Some(slot);
        changed
    }

    /// Row `row`'s cell `column` as the movie writes it.
    pub fn text(&self, row: usize, column: usize) -> String {
        self.0[row][column].map_or_else(|| EMPTY.to_owned(), Slot::name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The footage's table (1:46-2:04): name, primary, secondary.
    const FOOTAGE: [(&str, &str, &str); 24] = [
        ("Move Forward", "W", "EMPTY"),
        ("Move Backward", "S", "EMPTY"),
        ("Strafe Left", "A", "EMPTY"),
        ("Strafe Right", "D", "EMPTY"),
        ("Primary Attack", "MOUSE1", "EMPTY"),
        ("Primary Switch", "TAB", "MWHEEL"),
        ("Reload", "R", "EMPTY"),
        ("Secondary Attack", "MOUSE2", "G"),
        ("Secondary Switch", "V", "EMPTY"),
        ("Melee Attack", "F", "MOUSE3"),
        ("Use", "E", "EMPTY"),
        ("Jump", "SPACE", "EMPTY"),
        ("Crouch", "C", "EMPTY"),
        ("Binoculars", "B", "EMPTY"),
        ("PDA", "F1", "EMPTY"),
        ("Look Up", "I", "EMPTY"),
        ("Look Down", "K", "EMPTY"),
        ("Look Left", "J", "EMPTY"),
        ("Look Right", "L", "EMPTY"),
        ("Support Up", "1", "UP"),
        ("Support Down", "2", "DOWN"),
        ("View Switch", "4", "RIGHT"),
        ("Walk", "LCTRL", "EMPTY"),
        ("Sprint", "LSHIFT", "EMPTY"),
    ];

    fn shown(text: String) -> String {
        if text == EMPTY { "EMPTY".into() } else { text }
    }

    #[test]
    fn the_defaults_are_the_footages_table() {
        let t = KeyTable::default();
        for (i, (name, primary, secondary)) in FOOTAGE.iter().enumerate() {
            assert_eq!(shown(t.text(i, 0)), *primary, "{name}");
            assert_eq!(shown(t.text(i, 1)), *secondary, "{name}");
        }
    }

    #[test]
    fn the_names_are_text_ids() {
        for row in ROWS {
            assert!(row.name.starts_with('[') && row.name.ends_with(']'));
            assert!(!row.actions.is_empty());
        }
    }

    #[test]
    fn applying_the_defaults_changes_nothing_the_table_shows() {
        let mut b = Bindings::default();
        KeyTable::default().apply(&mut b);
        assert_eq!(KeyTable::of(&b), KeyTable::default());
        assert_eq!(
            b.0[&Action::SwitchWeapon],
            vec![
                Control::Key(KeyCode::Tab),
                Mouse::WheelUp.into(),
                Mouse::WheelDown.into()
            ]
        );
    }

    #[test]
    fn a_key_moves_from_the_cell_it_was_in() {
        let mut t = KeyTable::default();
        // Jump's secondary cell takes W from Move Forward.
        let changed = t.set(11, 1, Slot::Control(KeyCode::KeyW.into()));
        assert_eq!(changed, vec![11, 0]);
        assert_eq!(t.text(11, 1), "W");
        assert_eq!(t.text(0, 0), EMPTY);
        let mut b = Bindings::default();
        t.apply(&mut b);
        assert!(b.0[&Action::MoveForward].is_empty());
        assert_eq!(
            b.0[&Action::MoveUp],
            vec![KeyCode::Space.into(), KeyCode::KeyW.into()]
        );
    }

    #[test]
    fn a_row_moves_all_its_actions() {
        let mut t = KeyTable::default();
        t.set(12, 0, Slot::Control(KeyCode::KeyQ.into()));
        let mut b = Bindings::default();
        t.apply(&mut b);
        assert_eq!(b.0[&Action::MoveDown], vec![KeyCode::KeyQ.into()]);
        assert_eq!(b.0[&Action::Aim], vec![KeyCode::KeyQ.into()]);
    }

    #[test]
    fn keys_are_written_as_the_footage_writes_them() {
        let name = |k: KeyCode| Slot::Control(k.into()).name();
        assert_eq!(name(KeyCode::KeyW), "W");
        assert_eq!(name(KeyCode::Digit4), "4");
        assert_eq!(name(KeyCode::F1), "F1");
        assert_eq!(name(KeyCode::ControlLeft), "LCTRL");
        assert_eq!(name(KeyCode::ArrowRight), "RIGHT");
        assert_eq!(Slot::from(Control::from(Mouse::WheelDown)).name(), "MWHEEL");
        assert_eq!(Slot::Control(Mouse::Middle.into()).name(), "MOUSE3");
    }
}
