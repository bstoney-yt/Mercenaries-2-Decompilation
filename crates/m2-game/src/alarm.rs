//! Alarm towers: the engine's side of an alarm being raised or lowered.
//!
//! From the 360 code:
//!
//! - Objects with the `Alarm` component (the component set up at
//!   823B2E90) each keep one flag in the engine, the alarm being on (the
//!   store at 83657E5C). The tower templates carry it
//!   (`vzoutpost_bld_alarmtower` and the others), and their object script
//!   is `Alarm` (0xbe65fdd0).
//! - Raising, lowering or flipping an alarm is a message `{object, mode}`
//!   posted to a queue (8322E7A0, the push at 822B5490): mode 0 on, 1 off,
//!   2 flip. The queue holds 8; a message posted to a full queue is lost.
//!   `Junk.ToggleAlarm(object)` (82327380) posts a flip;
//!   `Junk.ActivateAlarm(object, on)` (823272D8) posts on for `true` and
//!   off for `false`, and needs both (a nil answer without); the AI's
//!   alarm task (822B4D70) posts on when its soldier reaches the tower.
//! - The game's loop (8221E340) takes the messages in order; each one
//!   (823976A8) whose object has no alarm flag does nothing; a mode above
//!   2 does nothing; otherwise it sets the flag (or flips it) and, when the
//!   object has an object script (the `ObjectScript` store, 83654784),
//!   calls the script's `OnUse(object, on)` with the flag as it now is.
//!   It is called for every message, whether the flag changed or not.
//!
//! The `Alarm` script does the rest: `OnUse` with `true` raises the alarm
//! (`AlarmActivated`: it turns on the spawners of the buildings labelled
//! `Occupied` within 100 m, `DangerousBuilding.TurnOn(buildings, true,
//! false, true)`, sounds the siren and lights the lamp, mutes the siren
//! after 60 s, and checks every 8 s whether one of those buildings is
//! still standing, lowering the alarm when none is); with `false` it
//! lowers it (`AlarmDeactivated`: siren, lamp and context action back).
//! The player raises it with the context action `[ContextAction.UseAlarm]`
//! (`Junk.ToggleAlarm`) and lowers it with the same prompt, which calls
//! `AlarmDeactivated` directly. Lowering the alarm does not stop the
//! buildings' spawners: `TurnOn` was not permanent, and only
//! `DangerousBuilding`'s `RemoveDB` (the building destroyed, or leaving
//! play) turns them off.
//!
//! Followed through, not checked in game: lowering the alarm with the
//! context action goes straight to the script and leaves the engine's
//! flag on. Taking the prompt again then flips the flag off, and `OnUse`
//! hears `false`: the tower stays quiet. A soldier only goes to raise an
//! alarm whose flag is off (822E1DD0).
//!
//! Inferred: that the flag starts off, and starts off again when the
//! object comes back into play (the component's own setup was not read;
//! the script's `OnActivate` starts with its lamp off). Where in the frame
//! the 360 takes the messages relative to the scripts' events (here: after
//! them, each tick).

use std::collections::HashMap;

/// How many messages the queue holds (8322E7A0: eight slots).
pub const QUEUE: usize = 8;

/// A message's mode: raise.
pub const ON: u32 = 0;
/// Lower.
pub const OFF: u32 = 1;
/// Flip.
pub const TOGGLE: u32 = 2;

/// The alarms in play and the messages waiting for them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Alarms {
    /// Each alarm object in play: whether it is on.
    pub on: HashMap<u32, bool>,
    /// Each alarm object's object script, by module hash, for those with one.
    pub scripts: HashMap<u32, u32>,
    /// Each alarm object's faction, as its `Association` names it
    /// (`Allied`, `China`, `Guerilla`), for those that name one: the
    /// soldiers who run to raise it are of it.
    pub factions: HashMap<u32, String>,
    /// Messages posted and not yet taken, in order: (object, mode).
    pub queue: Vec<(u32, u32)>,
}

impl Alarms {
    /// An alarm object comes into play, its alarm off, with its object
    /// script and its faction.
    pub fn add(&mut self, object: u32, script: Option<u32>, faction: Option<String>) {
        self.on.entry(object).or_insert(false);
        if let Some(s) = script {
            self.scripts.insert(object, s);
        }
        if let Some(f) = faction {
            self.factions.insert(object, f);
        }
    }

    /// It leaves play.
    pub fn remove(&mut self, object: u32) {
        self.on.remove(&object);
        self.scripts.remove(&object);
        self.factions.remove(&object);
    }

    /// Posts a message; lost when the queue is full.
    pub fn post(&mut self, object: u32, mode: u32) {
        if self.queue.len() < QUEUE {
            self.queue.push((object, mode));
        }
    }

    /// The messages waiting, taken.
    pub fn take(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.queue)
    }

    /// Carries out one message: the object's flag as it now is, for its
    /// script's `OnUse`; `None` for an object without an alarm or a mode
    /// that is none of the three.
    pub fn apply(&mut self, object: u32, mode: u32) -> Option<bool> {
        let on = self.on.get_mut(&object)?;
        *on = match mode {
            ON => true,
            OFF => false,
            TOGGLE => !*on,
            _ => return None,
        };
        Some(*on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_sets_or_flips_the_flag_and_answers_it_every_time() {
        let mut a = Alarms::default();
        a.add(7, None, None);
        assert_eq!(a.apply(7, TOGGLE), Some(true));
        // On again: still on, and its script hears it again.
        assert_eq!(a.apply(7, ON), Some(true));
        assert_eq!(a.apply(7, TOGGLE), Some(false));
        assert_eq!(a.apply(7, OFF), Some(false));
        assert_eq!(a.apply(7, 3), None);
        assert!(!a.on[&7]);
        // An object with no alarm: nothing.
        assert_eq!(a.apply(8, ON), None);
        assert!(!a.on.contains_key(&8));
        a.remove(7);
        assert_eq!(a.apply(7, ON), None);
    }

    #[test]
    fn the_queue_holds_eight_and_loses_the_rest() {
        let mut a = Alarms::default();
        for i in 0..10 {
            a.post(i, TOGGLE);
        }
        let taken = a.take();
        assert_eq!(taken.len(), QUEUE);
        assert_eq!(taken[0], (0, TOGGLE));
        assert_eq!(taken[7], (7, TOGGLE));
        assert!(a.take().is_empty());
    }

    #[test]
    fn an_alarm_coming_back_into_play_keeps_no_old_flag() {
        let mut a = Alarms::default();
        a.add(7, None, None);
        a.apply(7, ON);
        // Added twice while in play: unchanged.
        a.add(7, None, None);
        assert!(a.on[&7]);
        a.remove(7);
        a.add(7, None, None);
        assert!(!a.on[&7]);
    }
}
