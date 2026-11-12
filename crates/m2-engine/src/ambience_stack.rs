//! Which ambience plays: the game's short list of ambiences asked for, by
//! priority.
//!
//! Scripts (`Sound.CueAmbience`, `Sound.StopAmbience`), the ambience
//! regions the listener stands in and the interior they are inside each
//! ask for an ambience; the list keeps up to [`CAPACITY`] of them and the
//! one of the highest priority plays (the newest of equals). Anything that
//! wants an ambience heard calls [`AmbienceStack::cue`] and takes it back
//! with [`AmbienceStack::stop`]; the game calls [`AmbienceStack::update`]
//! once a frame and acts on the [`Change`] it answers.
//!
//! # Provenance
//!
//! From the 360 code (the ambience manager, the sound system's member at
//! +0x70):
//!
//! - Adding (`0x8236CFF8`): nothing when the list holds five; else the
//!   entry takes the first free of five slots and is linked in before the
//!   first entry of a greater priority, so after its equals.
//! - Removing (`0x8236D0C8`): the first entry in list order whose cue and
//!   overlay cue both match is unlinked.
//! - `Sound.CueAmbience(name)` (`0x82342F28`) adds the cue alone at
//!   priority 0 ([`SCRIPT_PRIORITY`]); `Sound.StopAmbience(name)`
//!   (`0x82342FB0`) removes the entry of that cue with no overlay.
//! - A region (`0x8236DBD8`) adds its `SoundAmbience` record's five words
//!   (cue, bank, priority, reverb preset, overlay cue) as the listener
//!   comes inside its shape and removes them (by cue and overlay) as they
//!   leave. An interior (`0x8236D148`) adds its cue, bank and reverb at
//!   priority 10 ([`INTERIOR_PRIORITY`]) with no overlay.
//! - The update (`0x8236D238`): the last entry of the list is the one to
//!   play. When its cue or overlay differs from what plays, the playing
//!   cue and overlay are stopped, the entry's bank is asked for, and
//!   (unless silenced) the entry's cue and overlay are started and its
//!   reverb preset, if it names one, is set. With the list empty and
//!   something playing, both are stopped and, if the playing entry had
//!   set a reverb, the default reverb is put back.
//! - `Sound.SilenceAmbience(flag)` (`0x8236CED0`): when the flag changes,
//!   the playing cue and overlay are stopped (true) or started (false).
//!   The list and what counts as playing are untouched.
//!
//! Every stop and start is sent with a fade time of -1.0 (`0x82DE896C`),
//! read here as "the cue's own"; how long that is has not been traced.

/// How many ambiences the list holds.
pub const CAPACITY: usize = 5;
/// The priority of an ambience a script cues.
pub const SCRIPT_PRIORITY: i32 = 0;
/// The priority of an interior's ambience.
pub const INTERIOR_PRIORITY: i32 = 10;

/// One ambience asked for. Cues, banks and presets are name hashes; 0 is
/// none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub cue: u32,
    /// The sound bank asked for as the entry starts to play.
    pub bank: u32,
    pub priority: i32,
    /// The reverb preset set as the entry starts to play, if not 0.
    pub reverb: u32,
    /// A second cue started and stopped with the first.
    pub overlay: u32,
}

impl Entry {
    /// What `Sound.CueAmbience` asks for.
    pub fn script(cue: u32) -> Self {
        Self {
            cue,
            priority: SCRIPT_PRIORITY,
            ..Self::default()
        }
    }

    /// What an interior asks for.
    pub fn interior(cue: u32, bank: u32, reverb: u32) -> Self {
        Self {
            cue,
            bank,
            priority: INTERIOR_PRIORITY,
            reverb,
            overlay: 0,
        }
    }
}

/// What the game is to do after an [`AmbienceStack::update`] or
/// [`AmbienceStack::silence`], in this order: stop, ask for the bank,
/// start, set the reverb.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// Cues to stop (the cue, then its overlay; 0s left out).
    pub stop: Vec<u32>,
    /// The bank to have loaded, if any.
    pub bank: Option<u32>,
    /// Cues to start (the cue, then its overlay; 0s left out).
    pub start: Vec<u32>,
    /// The reverb to set: a preset's hash, or `Some(None)` for the
    /// default put back.
    pub reverb: Option<Option<u32>>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn cues(cue: u32, overlay: u32) -> Vec<u32> {
    [cue, overlay].into_iter().filter(|&c| c != 0).collect()
}

/// The list, and what of it plays.
#[derive(Clone, Debug, Default)]
pub struct AmbienceStack {
    /// In list order: rising priority, the newest of equals last.
    entries: Vec<Entry>,
    /// The entry last started (or held back by silence).
    playing: Option<Entry>,
    silenced: bool,
}

impl AmbienceStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks for `entry`. False when the list is full, and it is dropped.
    pub fn cue(&mut self, entry: Entry) -> bool {
        if self.entries.len() >= CAPACITY {
            return false;
        }
        let at = self
            .entries
            .iter()
            .position(|e| e.priority > entry.priority)
            .unwrap_or(self.entries.len());
        self.entries.insert(at, entry);
        true
    }

    /// Takes back the first entry of `cue` and `overlay`. False if none.
    pub fn stop(&mut self, cue: u32, overlay: u32) -> bool {
        let Some(at) = self
            .entries
            .iter()
            .position(|e| e.cue == cue && e.overlay == overlay)
        else {
            return false;
        };
        self.entries.remove(at);
        true
    }

    /// The entry to play: the last of the list.
    pub fn active(&self) -> Option<&Entry> {
        self.entries.last()
    }

    /// The cue heard now: the playing entry's, unless silenced.
    pub fn heard(&self) -> Option<u32> {
        self.playing
            .filter(|_| !self.silenced)
            .map(|e| e.cue)
            .filter(|&c| c != 0)
    }

    pub fn is_silenced(&self) -> bool {
        self.silenced
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Brings what plays in line with the list.
    pub fn update(&mut self) -> Change {
        let mut change = Change::default();
        let (was_cue, was_overlay) = self.playing.map_or((0, 0), |p| (p.cue, p.overlay));
        match self.entries.last().copied() {
            None => {
                if was_cue != 0 || was_overlay != 0 {
                    if self.playing.is_some_and(|p| p.reverb != 0) {
                        change.reverb = Some(None);
                    }
                    change.stop = cues(was_cue, was_overlay);
                    self.playing = None;
                }
            }
            Some(e) => {
                if was_cue != e.cue || was_overlay != e.overlay {
                    change.stop = cues(was_cue, was_overlay);
                    change.bank = (e.bank != 0).then_some(e.bank);
                    if !self.silenced {
                        change.start = cues(e.cue, e.overlay);
                        if e.reverb != 0 {
                            change.reverb = Some(Some(e.reverb));
                        }
                    }
                    self.playing = Some(e);
                }
            }
        }
        change
    }

    /// `Sound.SilenceAmbience`: stops what plays, or starts it again.
    pub fn silence(&mut self, on: bool) -> Change {
        let mut change = Change::default();
        if on != self.silenced {
            let (cue, overlay) = self.playing.map_or((0, 0), |p| (p.cue, p.overlay));
            if on {
                change.stop = cues(cue, overlay);
            } else {
                change.start = cues(cue, overlay);
            }
        }
        self.silenced = on;
        change
    }
}

/// Which regions the listener is inside, so each asks for its ambience
/// once as they come in and takes it back as they leave.
///
/// From the 360 code (`0x8236DBD8`): each region keeps a flag, set as
/// the listener is found inside (whether or not the list had room) and
/// cleared as they are found outside, when its cue and overlay are
/// removed from the list.
#[derive(Clone, Debug, Default)]
pub struct Inside {
    /// The regions the listener is in, by entity, and what each asked for.
    regions: Vec<(u32, Entry)>,
}

impl Inside {
    /// The listener is now inside `now` (each region's entity and
    /// ambience): the regions left take theirs back from `stack`, those
    /// entered ask for theirs, in `now`'s order.
    pub fn listen(&mut self, stack: &mut AmbienceStack, now: &[(u32, Entry)]) {
        self.regions.retain(|(entity, e)| {
            let stays = now.iter().any(|n| n.0 == *entity);
            if !stays {
                stack.stop(e.cue, e.overlay);
            }
            stays
        });
        for &(entity, e) in now {
            if !self.regions.iter().any(|r| r.0 == entity) {
                stack.cue(e);
                self.regions.push((entity, e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_ask_as_the_listener_enters_and_take_back_as_they_leave() {
        let mut s = AmbienceStack::new();
        let mut inside = Inside::default();
        let (jungle, city) = ((1, region(100, 0)), (2, region(200, 0)));
        inside.listen(&mut s, &[jungle]);
        inside.listen(&mut s, &[jungle]);
        assert_eq!(s.len(), 1);
        // Walking into the city inside the jungle: the one entered last plays.
        inside.listen(&mut s, &[jungle, city]);
        assert_eq!(s.active().map(|e| e.cue), Some(200));
        // A script's cue at the same priority, later still, plays over both.
        s.cue(Entry::script(300));
        assert_eq!(s.active().map(|e| e.cue), Some(300));
        s.stop(300, 0);
        inside.listen(&mut s, &[jungle]);
        assert_eq!(s.active().map(|e| e.cue), Some(100));
        inside.listen(&mut s, &[]);
        assert!(s.is_empty());
    }

    fn region(cue: u32, priority: i32) -> Entry {
        Entry {
            cue,
            bank: cue,
            priority,
            ..Entry::default()
        }
    }

    #[test]
    fn the_highest_priority_plays_and_the_newest_of_equals() {
        let mut s = AmbienceStack::new();
        assert!(s.cue(region(1, 0)));
        assert!(s.cue(Entry::interior(2, 20, 0)));
        assert!(s.cue(Entry::script(3)));
        assert_eq!(s.active().map(|e| e.cue), Some(2));
        assert!(s.stop(2, 0));
        // 3 came after 1 at the same priority.
        assert_eq!(s.active().map(|e| e.cue), Some(3));
        assert!(s.stop(3, 0));
        assert_eq!(s.active().map(|e| e.cue), Some(1));
    }

    #[test]
    fn a_sixth_is_dropped() {
        let mut s = AmbienceStack::new();
        for c in 1..=5 {
            assert!(s.cue(Entry::script(c)));
        }
        assert!(!s.cue(Entry::interior(6, 0, 0)));
        assert_eq!(s.len(), CAPACITY);
        assert_eq!(s.active().map(|e| e.cue), Some(5));
    }

    #[test]
    fn stopping_matches_cue_and_overlay_and_takes_the_first() {
        let mut s = AmbienceStack::new();
        let with_overlay = Entry {
            overlay: 9,
            ..region(1, 0)
        };
        s.cue(with_overlay);
        s.cue(region(1, 5));
        assert!(!s.stop(1, 7));
        // A script's stop names no overlay: the plain entry goes.
        assert!(s.stop(1, 0));
        assert_eq!(s.active(), Some(&with_overlay));
        assert!(!s.stop(1, 0));
        assert!(s.stop(1, 9));
        assert!(s.is_empty());
    }

    #[test]
    fn a_change_of_the_active_entry_stops_loads_starts_and_sets_reverb() {
        let mut s = AmbienceStack::new();
        s.cue(region(1, 0));
        assert_eq!(
            s.update(),
            Change {
                stop: vec![],
                bank: Some(1),
                start: vec![1],
                reverb: None,
            }
        );
        assert!(s.update().is_empty());
        s.cue(Entry {
            overlay: 8,
            reverb: 5,
            ..Entry::interior(2, 20, 0)
        });
        assert_eq!(
            s.update(),
            Change {
                stop: vec![1],
                bank: Some(20),
                start: vec![2, 8],
                reverb: Some(Some(5)),
            }
        );
        assert_eq!(s.heard(), Some(2));
        // The same cue asked for again at another priority changes nothing.
        s.cue(Entry {
            overlay: 8,
            priority: 50,
            ..Entry::script(2)
        });
        assert!(s.update().is_empty());
    }

    #[test]
    fn an_empty_list_stops_what_plays_and_puts_the_reverb_back() {
        let mut s = AmbienceStack::new();
        s.cue(Entry::interior(2, 20, 5));
        s.update();
        s.stop(2, 0);
        assert_eq!(
            s.update(),
            Change {
                stop: vec![2],
                reverb: Some(None),
                ..Change::default()
            }
        );
        assert_eq!(s.heard(), None);
        assert!(s.update().is_empty());
        // Without a reverb of its own, none is put back.
        s.cue(Entry::script(3));
        s.update();
        s.stop(3, 0);
        assert_eq!(s.update().reverb, None);
    }

    #[test]
    fn silence_stops_and_restarts_what_plays_without_touching_the_list() {
        let mut s = AmbienceStack::new();
        s.cue(region(1, 0));
        s.update();
        assert_eq!(s.silence(true).stop, vec![1]);
        assert!(s.silence(true).is_empty());
        assert_eq!(s.heard(), None);
        // A change while silenced loads the bank and stops, but starts
        // nothing and sets no reverb.
        s.cue(Entry::interior(2, 20, 5));
        assert_eq!(
            s.update(),
            Change {
                stop: vec![1],
                bank: Some(20),
                ..Change::default()
            }
        );
        assert_eq!(s.silence(false).start, vec![2]);
        assert_eq!(s.heard(), Some(2));
    }
}
