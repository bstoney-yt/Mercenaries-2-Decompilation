//! Her vehicle disguise as the 360 runs it: the player record's disguise
//! block (+0x398), filled while she drives a disguise vehicle with no one
//! close enough to see through it, drained while someone is.
//!
//! From the 360 code:
//! - **The vehicle's numbers** (`VehicleDisguiseScale`, store 0x83653B04,
//!   named by 0x823B3F50; three floats at +0/+4/+8, hashes 0xDAAE14B4,
//!   0x260FFA47, 0xF550BFA1): how near an AI must be to count (block [1],
//!   default 30 m), how long the gauge takes to fill (block [2], default
//!   10 s) and to drain (block [3], default 40 s). Loaded as she gets in
//!   (0x824461D8; defaults 0x820BB1F8, 0x820BAEEC, 0x820BAFA8).
//! - **Who sees her** (block [7..], at most 8, 0x82445EC8): an AI sensing
//!   her within block [1] (0x822DF988) adds itself, or refreshes its
//!   entry, with a value and 3 s on its timer (0x820C7A20); a full list
//!   gives up the entry with the largest value. An entry runs out after
//!   its 3 s (0x824460F8) and goes when the AI's sense of her resets
//!   (0x82446008). Each coming and going is told to the script callback at
//!   0x83134548 (0x82446790: the player, the AI, whether it came): the
//!   `InvestigatorCallback` a script hands `Disguise.Init` (0x82340B20;
//!   the mission scripts pass `HandleInvestigator`).
//! - **Each tick** (0x82445C38, one player):
//!   - In a disguise vehicle with no AI aboard (0x824463F0, 0x82446330):
//!     an entry valued under 30 (0x820BB1F8) holds the grace timer
//!     (block [4]) at 2 s; else it runs down and, at 0, lets the gauge
//!     fill (flag 0x20). Out of one (or with an AI aboard) she drops the
//!     disguise (0x824457F8: gauge 0, grace 2 s, no filling, undisguised).
//!   - With anyone on the list the gauge drains 100 / [3] a second and
//!     can't fill; else, free to fill and short of 100, it fills 100 / [2]
//!     a second. At 100 she is disguised (flag 0x40); at 0 she isn't and
//!     can't fill. Kept to 0..100.
//!   - How long she has been disguised (block [6]) counts up to 5 s, and
//!     back to 0 as it ends.
//!   - A change of disguise calls the `Player.VehicleDisguise` callback
//!     (0x824466A8) and has every AI look at her afresh.
//! - **Getting in** (0x824461D8 -> 0x82446568): the gauge starts from 0,
//!   free to fill, unless the player is marked in the table at
//!   0x831342A8 +0x3CE4 (its entry's +0x4C), when she starts undisguised.
//!
//! Not settled (inferred or left out): the entry's value is what the AI
//! sensing passes (read as her distance, not confirmed), so what the
//! under-30 test means; the extra gate in 0x822DF988 (AI +0x17DC bit 0,
//! +0xFF0/+0xFF4: many writers, not traced); what marks the player in
//! 0x831342A8 +0x3CE4. The callback's latch (+0x438) is cleared only by
//! the reset on getting in (0x82445648).

/// Seconds an AI's sighting holds (0x820C7A20).
pub const SIGHTING: f32 = 3.0;
/// The most AIs the list keeps.
pub const MAX_WATCHERS: usize = 8;
/// An entry valued under this holds the grace timer (0x820BB1F8).
pub const CLOSE: f32 = 30.0;
/// The grace timer's length, s (0x820C77A4).
pub const GRACE: f32 = 2.0;
/// The gauge's top.
pub const FULL: f32 = 100.0;

/// A vehicle's `VehicleDisguiseScale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisguiseScale {
    /// How near an AI must be to count, m (block [1]).
    pub radius: f32,
    /// Seconds to fill, and to drain (blocks [2], [3]).
    pub fill: f32,
    pub drain: f32,
}

impl Default for DisguiseScale {
    /// 0x824461D8's defaults for a vehicle without one.
    fn default() -> Self {
        Self {
            radius: 30.0,
            fill: 10.0,
            drain: 40.0,
        }
    }
}

/// One AI that sees her (block [7..], 12 bytes each).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Watcher {
    pub who: u32,
    pub value: f32,
    pub timer: f32,
}

/// What a step or report tells the scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisguiseEvent {
    /// Disguised or not now (the `Player.VehicleDisguise` callback).
    Changed(bool),
    /// An AI came onto the list, or left it (the 0x83134548 callback).
    Watcher { who: u32, came: bool },
    /// Every AI looks at her afresh (0x822F87C0, +0x17DD bit 0x40).
    LookAgain,
}

/// The disguise block.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleDisguise {
    pub scale: DisguiseScale,
    pub grace: f32,
    /// The gauge, 0..=100 (block [5]).
    pub level: f32,
    /// Seconds disguised, up to 5 (block [6]).
    pub held_for: f32,
    pub watchers: Vec<Watcher>,
    /// Flag 0x20: the gauge may fill.
    pub may_fill: bool,
    /// Flag 0x40.
    pub disguised: bool,
}

impl VehicleDisguise {
    /// 0x824461D8: she gets into a vehicle with `scale` (`None`: the
    /// defaults); `marked`, the player's entry in 0x831342A8 +0x3CE4.
    ///
    /// The block is reset first (0x82445648: flags 0x80, 0x40 and 0x20
    /// cleared, the list emptied, the gauge 0, the grace 2 s, the
    /// callback's latch cleared), so getting in never fires the callback.
    pub fn enter(&mut self, scale: Option<DisguiseScale>, marked: bool) -> Vec<DisguiseEvent> {
        self.scale = scale.unwrap_or_default();
        self.watchers.clear();
        self.disguised = false;
        self.may_fill = false;
        self.level = 0.0;
        self.grace = GRACE;
        let mut out = Vec::new();
        if marked {
            self.drop_disguise(&mut out);
        } else {
            self.grace = 0.0;
            self.may_fill = true;
        }
        out
    }

    /// 0x82445EC8: an AI sensing her within the radius, with `value`.
    pub fn sighted(&mut self, who: u32, value: f32) -> Vec<DisguiseEvent> {
        let mut out = Vec::new();
        let slot = match self.watchers.iter().position(|w| w.who == who) {
            Some(i) => i,
            None if self.watchers.len() < MAX_WATCHERS => {
                self.watchers.push(Watcher {
                    who,
                    value,
                    timer: SIGHTING,
                });
                out.push(DisguiseEvent::Watcher { who, came: true });
                return out;
            }
            None => {
                // The one with the largest value gives way.
                let (i, w) = self
                    .watchers
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.value.total_cmp(&b.1.value))
                    .expect("full");
                out.push(DisguiseEvent::Watcher {
                    who: w.who,
                    came: false,
                });
                out.push(DisguiseEvent::Watcher { who, came: true });
                i
            }
        };
        self.watchers[slot] = Watcher {
            who,
            value,
            timer: SIGHTING,
        };
        out
    }

    /// 0x82446008: an AI's sense of her reset.
    pub fn forget(&mut self, who: u32) -> Vec<DisguiseEvent> {
        match self.watchers.iter().position(|w| w.who == who) {
            Some(i) => {
                self.watchers.remove(i);
                vec![DisguiseEvent::Watcher { who, came: false }]
            }
            None => Vec::new(),
        }
    }

    fn close(&self) -> bool {
        self.watchers.iter().any(|w| w.value < CLOSE)
    }

    /// 0x824457F8.
    fn drop_disguise(&mut self, out: &mut Vec<DisguiseEvent>) {
        if self.disguised || (!self.close() && self.may_fill) {
            out.push(DisguiseEvent::LookAgain);
        }
        self.level = 0.0;
        self.grace = GRACE;
        self.may_fill = false;
        self.disguised = false;
    }

    /// 0x82445C38: one tick of `dt`; `in_disguise_vehicle`: she drives a
    /// vehicle with no AI aboard (0x824463F0's test).
    pub fn tick(&mut self, dt: f32, in_disguise_vehicle: bool) -> Vec<DisguiseEvent> {
        let mut out = Vec::new();
        if dt == 0.0 {
            return out;
        }
        let was = self.disguised;
        // 0x824463F0.
        if in_disguise_vehicle {
            if self.close() {
                self.grace = GRACE;
            } else {
                self.grace -= dt;
                if self.grace <= 0.0 {
                    self.grace = 0.0;
                    self.may_fill = true;
                }
            }
        } else {
            self.drop_disguise(&mut out);
        }
        let rate = if !self.watchers.is_empty() {
            self.may_fill = false;
            -FULL / self.scale.drain
        } else if self.may_fill && self.level < FULL {
            FULL / self.scale.fill
        } else {
            0.0
        };
        self.level += rate * dt;
        if self.level >= FULL {
            self.disguised = true;
        }
        if self.level <= 0.0 {
            if was {
                out.push(DisguiseEvent::LookAgain);
            }
            self.may_fill = false;
            self.disguised = false;
        }
        self.level = self.level.clamp(0.0, FULL);
        // 0x824460F8: sightings run out.
        let mut i = 0;
        while i < self.watchers.len() {
            self.watchers[i].timer -= dt;
            if self.watchers[i].timer <= 0.0 {
                let w = self.watchers.remove(i);
                out.push(DisguiseEvent::Watcher {
                    who: w.who,
                    came: false,
                });
            } else {
                i += 1;
            }
        }
        if self.held_for < 5.0 {
            self.held_for += dt;
        }
        if self.disguised != was {
            if !self.disguised {
                self.held_for = 0.0;
            }
            out.push(DisguiseEvent::Changed(self.disguised));
            out.push(DisguiseEvent::LookAgain);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(d: &mut VehicleDisguise, seconds: f32, in_vehicle: bool) -> Vec<DisguiseEvent> {
        let mut out = Vec::new();
        for _ in 0..(seconds * 10.0).round() as usize {
            out.extend(d.tick(0.1, in_vehicle));
        }
        out
    }

    #[test]
    fn unseen_in_a_disguise_vehicle_the_gauge_fills_and_she_is_disguised() {
        let mut d = VehicleDisguise::default();
        assert!(d.enter(None, false).is_empty());
        // 10 s to fill at the default scale.
        let out = run(&mut d, 9.5, true);
        assert!(!d.disguised && out.is_empty(), "{}", d.level);
        let out = run(&mut d, 1.0, true);
        assert!(d.disguised);
        assert!(out.contains(&DisguiseEvent::Changed(true)));
        assert_eq!(d.level, FULL);
    }

    #[test]
    fn someone_near_drains_it_and_holds_the_grace() {
        let mut d = VehicleDisguise::default();
        d.enter(None, false);
        run(&mut d, 11.0, true);
        assert!(d.disguised);
        // A close AI keeps refreshing: 40 s to drain.
        let mut out = Vec::new();
        for _ in 0..400 {
            out.extend(d.sighted(7, 10.0));
            out.extend(d.tick(0.1, true));
        }
        assert!(!d.disguised && d.level == 0.0);
        assert!(out.contains(&DisguiseEvent::Changed(false)));
        assert_eq!(d.grace, GRACE);
        // It goes 3 s after its last sighting, and the grace then runs 2 s
        // before the gauge may fill again.
        let out = run(&mut d, 3.0, true);
        assert!(out.contains(&DisguiseEvent::Watcher {
            who: 7,
            came: false
        }));
        assert!(!d.may_fill);
        run(&mut d, 2.1, true);
        assert!(d.may_fill);
    }

    #[test]
    fn out_of_the_vehicle_it_drops_and_the_list_keeps_eight() {
        let mut d = VehicleDisguise::default();
        d.enter(None, false);
        run(&mut d, 11.0, true);
        let out = d.tick(0.1, false);
        assert!(!d.disguised && d.level == 0.0 && d.grace == GRACE);
        assert!(out.contains(&DisguiseEvent::Changed(false)));
        for who in 0..8 {
            d.sighted(who, who as f32 * 5.0);
        }
        // Full: the one valued highest (7, at 35) gives way.
        let out = d.sighted(100, 1.0);
        assert_eq!(
            out,
            [
                DisguiseEvent::Watcher {
                    who: 7,
                    came: false
                },
                DisguiseEvent::Watcher {
                    who: 100,
                    came: true
                }
            ]
        );
        assert_eq!(d.watchers.len(), MAX_WATCHERS);
        assert_eq!(
            d.forget(3),
            [DisguiseEvent::Watcher {
                who: 3,
                came: false
            }]
        );
        // Marked as she gets in: undisguised from the start.
        let mut m = VehicleDisguise::default();
        m.enter(None, true);
        assert!(!m.may_fill && m.grace == GRACE);
        // Getting in again resets the block and says nothing.
        assert!(d.enter(None, false).is_empty());
        assert!(d.watchers.is_empty() && d.may_fill && d.grace == 0.0);
    }
}
