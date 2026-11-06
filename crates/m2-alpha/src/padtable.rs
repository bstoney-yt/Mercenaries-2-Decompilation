//! The pause menu's and the front end's joystick table (Options,
//! Controls, Joystick): the keyboard table's 24 rows
//! ([`crate::keytable::ROWS`]), each with one pad button, and what the
//! pad's layouts ([`crate::pad::ON_FOOT`], [`crate::pad::DRIVING`]) become
//! under it.
//!
//! The movie (`joystick_mc`) shares `LTIInputKMTable` with the keyboard
//! screen: column 0 the actions' names, column 2 the buttons
//! (`LTIInputJoystickMap(row, text)`), which is the keyboard's secondary
//! column. So with no joystick the screen says "No Joystick Connected..."
//! and lists the keyboard's secondary keys, as the PC recording does
//! (t=82-88); the engine fills the column only when it has a joystick to
//! list ([`crate::pause::JoyScreen`]).
//!
//! # Provenance
//!
//! From the game's data: the movie's commands (`LTIInputJoystickEnter`,
//! `ChangeInput` with the row, `Cancel`, `ApplyChanges`, `Default`,
//! `Exit`, `ReEnter`, `ChangePrimary`) and the calls it takes
//! (`LTIInputNumberControllers`, `LTIInputControllerNames`,
//! `LTIInputSeeButtonsPushed`, `LTIFinishedGettingJoystickInput`,
//! `LTIInputJoystickCancel`, `LTIInputJoystickUpdateTable`).
//! From the PC recording: only the no-joystick screen.
//! Invented, since neither the recording nor the 360 code shows a joystick
//! on this screen: the buttons' names and [`PAD_NAME`]; the default
//! table (the pad's own layout, [`crate::pad::ON_FOOT`], with the
//! stick's rows empty and Sprint on L3); that a row changed moves in both
//! layouts; that any key or mouse button calls off the wait for a button.

use crate::input::Action;
use crate::keytable::{EMPTY, ROWS};
use crate::pad::{Button, ON_FOOT};

/// What the screen calls the pad (invented: XInput gives no name).
pub const PAD_NAME: &str = "Xbox Controller";

/// The Sprint row ([`crate::pad::Pad`] latches it itself).
const SPRINT_ROW: usize = 23;

/// The table: each row's button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadTable(pub [Option<Button>; ROWS.len()]);

impl Default for PadTable {
    /// The pad's own layout.
    fn default() -> Self {
        let mut cells = [None; ROWS.len()];
        for (row, cell) in ROWS.iter().zip(&mut cells) {
            *cell = ON_FOOT
                .iter()
                .find(|(_, a)| row.actions.contains(a))
                .map(|&(b, _)| b);
        }
        cells[SPRINT_ROW] = Some(Button::LeftStick);
        Self(cells)
    }
}

impl PadTable {
    /// The table as the settings file lists it, by the buttons' names: a
    /// list that isn't the table's is the defaults, an unknown name
    /// empty.
    pub fn from_names(names: &[String]) -> Self {
        let mut t = Self::default();
        if names.len() == ROWS.len() {
            for (cell, name) in t.0.iter_mut().zip(names) {
                *cell = Button::by_name(name);
            }
        }
        t
    }

    /// The buttons' names for the settings file, empty for the defaults.
    pub fn names(&self) -> Vec<String> {
        if *self == Self::default() {
            return Vec::new();
        }
        self.0
            .iter()
            .map(|b| b.map_or(String::new(), |b| b.name().to_owned()))
            .collect()
    }

    /// Puts `button` in `row`, taking it out of any other: the rows
    /// changed.
    pub fn set(&mut self, row: usize, button: Button) -> Vec<usize> {
        let mut changed = vec![row];
        for (r, cell) in self.0.iter_mut().enumerate() {
            if *cell == Some(button) && r != row {
                *cell = None;
                changed.push(r);
            }
        }
        self.0[row] = Some(button);
        changed
    }

    /// The row holding `button`.
    pub fn holder(&self, button: Button) -> Option<usize> {
        self.0.iter().position(|b| *b == Some(button))
    }

    /// Row `row` as the movie writes it.
    pub fn text(&self, row: usize) -> String {
        self.0[row].map_or_else(|| EMPTY.to_owned(), |b| b.name().to_owned())
    }

    /// The button that latches sprint.
    pub fn sprint(&self) -> Option<Button> {
        self.0[SPRINT_ROW]
    }

    /// `base` with the rows changed from the defaults moved to their
    /// buttons (or gone, emptied); what isn't in a row stays.
    pub fn layout(&self, base: &[(Button, Action)]) -> Vec<(Button, Action)> {
        let default = Self::default();
        let changed = |r: usize| self.0[r] != default.0[r];
        let row_of = |a: Action| ROWS.iter().position(|row| row.actions.contains(&a));
        let mut out: Vec<(Button, Action)> = base
            .iter()
            .copied()
            .filter(|&(_, a)| !row_of(a).is_some_and(changed))
            .collect();
        for (r, row) in ROWS.iter().enumerate() {
            if let (true, Some(button), false) = (changed(r), self.0[r], r == SPRINT_ROW) {
                out.extend(row.actions.iter().map(|&a| (button, a)));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pad::DRIVING;

    #[test]
    fn the_defaults_are_the_pads_layout() {
        let t = PadTable::default();
        assert_eq!(t.layout(ON_FOOT), ON_FOOT.to_vec());
        assert_eq!(t.layout(DRIVING), DRIVING.to_vec());
        assert_eq!(t.text(4), "RT");
        assert_eq!(t.text(11), "A");
        assert_eq!(t.text(23), "L3");
        assert_eq!(t.text(0), EMPTY);
        assert_eq!(t.names(), Vec::<String>::new());
    }

    #[test]
    fn a_button_moves_from_the_row_it_was_in() {
        let mut t = PadTable::default();
        // Primary Attack takes A from Jump.
        assert_eq!(t.set(4, Button::A), vec![4, 11]);
        assert_eq!(t.text(4), "A");
        assert_eq!(t.text(11), EMPTY);
        let foot = t.layout(ON_FOOT);
        assert!(foot.contains(&(Button::A, Action::Fire)));
        assert!(!foot.contains(&(Button::RightTrigger, Action::Fire)));
        assert!(!foot.iter().any(|&(_, a)| a == Action::MoveUp));
        // Driving, Fire moved as well; the pedals stay.
        let car = t.layout(DRIVING);
        assert!(car.contains(&(Button::A, Action::Fire)));
        assert!(car.contains(&(Button::A, Action::MoveForward)));
    }

    #[test]
    fn a_row_moves_all_its_actions() {
        let mut t = PadTable::default();
        t.set(12, Button::RightBumper);
        let foot = t.layout(ON_FOOT);
        assert!(foot.contains(&(Button::RightBumper, Action::MoveDown)));
        assert!(foot.contains(&(Button::RightBumper, Action::Aim)));
        assert!(!foot.iter().any(|&(b, _)| b == Button::LeftTrigger));
    }

    #[test]
    fn an_empty_row_takes_its_first_button() {
        let mut t = PadTable::default();
        t.set(21, Button::LeftBumper);
        assert!(
            t.layout(ON_FOOT)
                .contains(&(Button::LeftBumper, Action::ViewSwitch))
        );
    }

    #[test]
    fn the_settings_file_keeps_the_table() {
        let mut t = PadTable::default();
        t.set(9, Button::DpadRight);
        let back = PadTable::from_names(&t.names());
        assert_eq!(back, t);
        assert_eq!(back.text(9), "DPAD RIGHT");
        assert_eq!(PadTable::from_names(&[]), PadTable::default());
    }

    #[test]
    fn sprint_is_the_pads_own_latch() {
        let mut t = PadTable::default();
        t.set(23, Button::X);
        assert_eq!(t.sprint(), Some(Button::X));
        assert!(!t.layout(ON_FOOT).iter().any(|&(_, a)| a == Action::Sprint));
    }
}
