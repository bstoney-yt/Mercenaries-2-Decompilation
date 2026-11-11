//! What a person plays standing still: the idle, the second idle they
//! settle into, and the fidget, as the original's locomotion node times
//! them.
//!
//! Standing and not aiming, a person settles from the idle into a second
//! idle after [`SETTLE`] seconds (where the tables give them one), and
//! every [`FIDGET_MIN`] to [`FIDGET_MAX`] seconds plays the fidget once
//! and settles back. Aiming puts them back in the plain idle and starts
//! both waits again.
//!
//! # Provenance
//!
//! From the 360 code: the locomotion node's standing step (0x82809D60),
//! its timer reset (0x82806B48) and its binder (0x82804AE8). The node's
//! `LooseMovementNodes` blend holds the idle (entry 11 of the stance's
//! handle list), the second idle (entry 12) and the fidget (entry 13;
//! `Upright_Fidget` by its hash) beside the gaits. The standing mode keeps
//! a state (+0x38: 0 idle, 3 second idle, 4 fidget; 1 and 2 are the aim
//! shuffles) and two timers:
//!
//! - the fidget's (+0x1590), reset to `15 + r * 15` seconds for a random
//!   `r` in 0..1 (the floats 15 and 30 at 0x8206F94C and 0x8206F950);
//! - the second idle's (+0x1594), reset to 3 seconds (0x8206F948).
//!
//! Both are reset on entering the standing mode, and each step the person
//! aims or is held (bit 0x20 of their record's flags), in the idle state.
//! Each change fades the old clip out and the new in over 0.3 seconds
//! ([`crate::blend::GAIT`], the float at 0x8206F940), the new one from
//! its start. [`StandIdle::step`] is the three states' steps:
//!
//! - **Idle**: the timers run down unless aiming or held. The fidget's
//!   out: the fidget. Else, with a second idle and its timer out: the
//!   second idle.
//! - **Second idle**: back to the idle, timers reset, when the person
//!   aims, or the second idle's timer is above 0, or their aim is turned
//!   more than [`SETTLED_AIM`] from their body (2 degrees: the float 2.0
//!   at 0x82003CF4 times the degree at 0x82E75080). Else the fidget's
//!   timer runs down, and out, the fidget.
//! - **Fidget**: played once. When it ends, or the person aims: to the
//!   second idle with its timer at 0 if they have one, else to the idle;
//!   timers reset.
//!
//! "Has a second idle" is the node's flag +0x15EF: entry 12's clip for
//! the person is not the table's stand-in
//! ([`m2_assets::animation_table::NO_ANIMATION`]).
//!
//! **Not traced**: what sets the held bit, and a message (0x238009A9)
//! that also resets the timers. The node sets bit 0x80 of the person's
//! flags while the fidget plays; what reads it is not traced.

/// The fidget comes this long after the timers start, at the soonest and
/// at the latest, seconds.
pub const FIDGET_MIN: f32 = 15.0;
pub const FIDGET_MAX: f32 = 30.0;

/// The second idle comes this long after the timers start, seconds.
pub const SETTLE: f32 = 3.0;

/// Turned further than this from the body, the aim ends the second idle,
/// radians (2 degrees).
pub const SETTLED_AIM: f32 = 2.0 * std::f32::consts::PI / 180.0;

/// What plays standing still.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Standing {
    /// The stance's idle.
    #[default]
    Idle,
    /// The second idle.
    Settled,
    /// The fidget, once.
    Fidget,
}

/// What the standing step reads of the person, each step.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Person {
    pub aiming: bool,
    /// Bit 0x20 of their record's flags.
    pub held: bool,
    /// The tables give them a second idle of their own.
    pub has_settled: bool,
    /// Their aim's turn from their body, radians.
    pub aim_yaw: f32,
    /// The fidget clip has played through.
    pub fidget_done: bool,
}

/// The standing mode's state and timers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StandIdle {
    pub state: Standing,
    /// Seconds until the fidget.
    pub fidget_in: f32,
    /// Seconds until the second idle.
    pub settle_in: f32,
}

impl StandIdle {
    /// Entering the standing mode: the idle, the timers started with the
    /// random `roll` (0..1).
    pub fn new(roll: f32) -> Self {
        let mut s = Self {
            state: Standing::Idle,
            fidget_in: 0.0,
            settle_in: 0.0,
        };
        s.reset(roll);
        s
    }

    /// Starts both timers again, the fidget's by `roll` (0..1).
    pub fn reset(&mut self, roll: f32) {
        self.fidget_in = FIDGET_MIN + roll.clamp(0.0, 1.0) * (FIDGET_MAX - FIDGET_MIN);
        self.settle_in = SETTLE;
    }

    /// One step of `dt` seconds. `roll` (0..1) is used if the timers are
    /// reset. Returns the new state when it changes: its clip is then to
    /// start from the beginning, fading in over [`crate::blend::GAIT`].
    pub fn step(&mut self, dt: f32, person: Person, roll: f32) -> Option<Standing> {
        let was = self.state;
        match self.state {
            Standing::Idle => {
                if person.aiming || person.held {
                    self.reset(roll);
                } else {
                    self.fidget_in -= dt;
                    self.settle_in -= dt;
                }
                if self.fidget_in <= 0.0 {
                    self.state = Standing::Fidget;
                } else if person.has_settled && self.settle_in <= 0.0 {
                    self.state = Standing::Settled;
                }
            }
            Standing::Settled => {
                if person.aiming || self.settle_in > 0.0 || person.aim_yaw.abs() > SETTLED_AIM {
                    self.reset(roll);
                    self.state = Standing::Idle;
                } else {
                    self.fidget_in -= dt;
                    if self.fidget_in <= 0.0 {
                        self.state = Standing::Fidget;
                    }
                }
            }
            Standing::Fidget => {
                if person.aiming || person.fidget_done {
                    self.reset(roll);
                    if person.has_settled {
                        self.settle_in = 0.0;
                        self.state = Standing::Settled;
                    } else {
                        self.state = Standing::Idle;
                    }
                }
            }
        }
        (self.state != was).then_some(self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STILL: Person = Person {
        aiming: false,
        held: false,
        has_settled: true,
        aim_yaw: 0.0,
        fidget_done: false,
    };

    /// Steps `s` for `seconds` in tenths, returning each change and when.
    fn run(s: &mut StandIdle, seconds: f32, person: Person) -> Vec<(u32, Standing)> {
        let mut out = Vec::new();
        for i in 1..=(seconds * 10.0).round() as u32 {
            if let Some(to) = s.step(0.1, person, 0.0) {
                out.push((i, to));
            }
        }
        out
    }

    #[test]
    fn the_fidget_waits_fifteen_to_thirty_seconds() {
        assert_eq!(StandIdle::new(0.0).fidget_in, 15.0);
        assert_eq!(StandIdle::new(0.5).fidget_in, 22.5);
        assert_eq!(StandIdle::new(1.0).fidget_in, 30.0);
        assert_eq!(StandIdle::new(7.0).fidget_in, 30.0);
        assert_eq!(StandIdle::new(0.3).settle_in, 3.0);
    }

    #[test]
    fn standing_still_she_settles_then_fidgets_and_settles_back() {
        let mut s = StandIdle::new(0.0);
        let changes = run(&mut s, 16.0, STILL);
        // Settled after 3 s; the fidget's 15 s ran from the start.
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].1, Standing::Settled);
        assert!((29..=31).contains(&changes[0].0), "{changes:?}");
        assert_eq!(changes[1].1, Standing::Fidget);
        assert!((149..=151).contains(&changes[1].0), "{changes:?}");
        // The fidget holds until its clip is done, then she is settled at
        // once (the second idle's timer at 0), the fidget's wait new.
        assert_eq!(run(&mut s, 5.0, STILL), []);
        let done = Person {
            fidget_done: true,
            ..STILL
        };
        assert_eq!(s.step(0.1, done, 1.0), Some(Standing::Settled));
        assert_eq!(s.settle_in, 0.0);
        assert_eq!(s.fidget_in, 30.0);
        assert_eq!(s.step(0.1, STILL, 0.0), None);
    }

    #[test]
    fn without_a_second_idle_she_only_fidgets() {
        let alone = Person {
            has_settled: false,
            ..STILL
        };
        let mut s = StandIdle::new(0.0);
        let changes = run(&mut s, 16.0, alone);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].1, Standing::Fidget);
        let done = Person {
            fidget_done: true,
            ..alone
        };
        assert_eq!(s.step(0.1, done, 0.0), Some(Standing::Idle));
        assert_eq!(s.settle_in, SETTLE);
    }

    #[test]
    fn aiming_keeps_her_in_the_idle_and_ends_the_others() {
        let aiming = Person {
            aiming: true,
            ..STILL
        };
        let mut s = StandIdle::new(0.0);
        assert_eq!(run(&mut s, 40.0, aiming), []);
        assert_eq!((s.fidget_in, s.settle_in), (15.0, 3.0));
        // Held is the same in the idle.
        let held = Person {
            held: true,
            ..STILL
        };
        assert_eq!(run(&mut s, 40.0, held), []);
        // Settled, then aiming: the idle, timers new.
        run(&mut s, 4.0, STILL);
        assert_eq!(s.state, Standing::Settled);
        assert_eq!(s.step(0.1, aiming, 0.5), Some(Standing::Idle));
        assert_eq!((s.fidget_in, s.settle_in), (22.5, 3.0));
        // In the fidget, aiming ends it as its end does.
        let mut f = StandIdle::new(0.0);
        f.state = Standing::Fidget;
        assert_eq!(f.step(0.1, aiming, 0.0), Some(Standing::Settled));
    }

    #[test]
    fn an_aim_turned_from_the_body_ends_the_second_idle() {
        let mut s = StandIdle::new(0.0);
        run(&mut s, 4.0, STILL);
        assert_eq!(s.state, Standing::Settled);
        let just = Person {
            aim_yaw: 1.9f32.to_radians(),
            ..STILL
        };
        assert_eq!(s.step(0.1, just, 0.0), None);
        let turned = Person {
            aim_yaw: -2.1f32.to_radians(),
            ..STILL
        };
        assert_eq!(s.step(0.1, turned, 0.0), Some(Standing::Idle));
    }
}
