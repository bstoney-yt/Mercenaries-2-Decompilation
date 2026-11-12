//! The things people use in passing (benches, chairs, telephones, food
//! carts, fountains): the template database's `SocialUse` records, and
//! the original's rules for when a thing calls someone over, whom, and
//! for how long they stay.
//!
//! In the original the thing picks the person. Each usable object counts
//! a timer down; when it runs out it takes the first of that frame's
//! idle people within its range and asks them to use it. They walk to a
//! free seat of the object and board it as they would a vehicle's (the
//! objects carry seats, and a seated person is in the `InVehicle`
//! stance), and leave it by the ordinary seat exit.
//!
//! # Provenance
//!
//! From samples: `SocialUse` is on 66 templates in 10 distinct records
//! of four fields ([`SocialUse`]): a kind, `Range`, a third (name
//! unknown) and `duration`. Benches are 2 / 8 m / 60 / 30 s, chairs
//! 2 / 5 / 60 / 20, telephones 4 / 8 / 60 / 25, food carts 0 / 5 / 300 /
//! 1, fountains 1 / 10 / 30 / 2, coin-operated things 5 / 10 / 30 / 5,
//! monuments 5 / 100 / 100 / 5.
//!
//! From the 360 code:
//!
//! - **The object's timer.** When a usable object comes in (0x82400440)
//!   it gets a timer record: the time left, a random share of the
//!   record's third field; that field again; and 5.0 (0x820BAF84). The
//!   living world's update (0x822F96B8) counts every such timer down and
//!   on its running out tries to recruit (0x822F94C0); the timer starts
//!   again at the third field if it took someone and at the 5.0 if not,
//!   plus up to [`JITTER`] (0x820BB49C): [`UseTimer`].
//! - **Recruiting** (0x822F94C0): of the frame's candidates, in order,
//!   the first whose distance to the object is under its `Range` is
//!   taken off the list and sent a request of type 0x1C for the object:
//!   [`recruit`].
//! - **Candidates** (0x822B1BD0, the planner's step for a person
//!   standing idle): at its end the person is added to the frame's list
//!   if their use meter is under [`CANDIDATE_BELOW`] and the list holds
//!   under [`CANDIDATES_MOST`]. The list is emptied every frame before
//!   the people think (0x822F8010).
//! - **The use meter** (0x822BDF30, each think of a planner person):
//!   while what they use has a `SocialUse` record and the meter is under
//!   [`METER_MOST`] it rises by [`METER_DONE`] over the record's
//!   `duration` each second (by [`METER_FAST`] a second for a duration
//!   of 1e-4 or less); otherwise it falls by [`METER_DRAIN`] a second to
//!   0. The use's step (0x822ABD60) ends the use, by the ordinary seat
//!   exit (0x822D2E80), once the meter is over [`METER_DONE`]:
//!   [`UseMeter`].
//! - **The request** (0x822DC468, type 0x1C): finds a free seat of the
//!   object (0x8229D840, the finder boarding a vehicle uses) and sets
//!   the planner's "used" goal; the use's step (0x822B4110) boards the
//!   seat (0x822D2DD8).
//! - **The lighter civilians' behaviour** `LivingWorldObject`
//!   (0x82302488, 0x82302650): walks to the seat, boards it, and stays
//!   the record's `duration` (5.0 with none) times 0.9 to 1.1:
//!   [`stay_seconds`].
//!
//! **Inferred**: the meter's steps are scaled by a field of the AI
//! (+0x508) read as the think's seconds. The third field's meaning, the
//! wait after a use, is from its use in the timer.
//!
//! **Not established**: what the kind selects (none of the functions
//! that read the record reads it); what happens at the things with no
//! seat (monuments, coin-operated); how a `LivingWorldObject` civilian
//! is given its object.

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};
use m2_math::glam::Vec3;

use crate::tuning::Table;

/// The record's fields with no known name: the kind and the wait after
/// a use.
const KIND: u32 = 0x0f00_1edd;
const REST: u32 = 0x41f5_57b1;

/// A timer that found nobody tries again after this, seconds.
pub const RETRY: f32 = 5.0;

/// Up to this is added to a timer's next wait, seconds.
pub const JITTER: f32 = 0.15;

/// A person idling is a candidate with their use meter under this.
pub const CANDIDATE_BELOW: f32 = 5.0;

/// The most candidates a frame's list holds.
pub const CANDIDATES_MOST: usize = 64;

/// The use meter: where a use is over, where it stops rising, what it
/// rises by a second for a use of no duration, and what it falls by a
/// second.
pub const METER_DONE: f32 = 10.0;
pub const METER_MOST: f32 = 15.0;
pub const METER_FAST: f32 = 20.0;
pub const METER_DRAIN: f32 = 0.1;

/// A `LivingWorldObject` civilian's stay at a thing with no record,
/// seconds.
pub const STAY_DEFAULT: f32 = 5.0;

/// A template's `SocialUse` record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SocialUse {
    /// The kind (0 food carts, 1 fountains, 2 benches and chairs, 4
    /// telephones, 5 coin-operated things and monuments; not read by
    /// the code found).
    pub kind: u32,
    /// How near a candidate must be to be called over, metres.
    pub range: f32,
    /// The object's wait after calling someone over, seconds.
    pub rest: f32,
    /// How long a use lasts, seconds.
    pub duration: f32,
}

/// Every template's `SocialUse` record, by template id.
pub fn read(db: &Layer<'_>) -> HashMap<u32, SocialUse> {
    let t = Table::new(db, "SocialUse");
    let (range, duration) = (name_hash("Range"), name_hash("duration"));
    t.templates()
        .filter_map(|id| {
            let r = *t.of_template(id).first()?;
            Some((
                id,
                SocialUse {
                    kind: t.word(r, KIND)?,
                    range: t.float(r, range)?,
                    rest: t.float(r, REST)?,
                    duration: t.float(r, duration)?,
                },
            ))
        })
        .collect()
}

/// A usable object's timer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UseTimer {
    /// Seconds left before it next tries to call someone over.
    pub left: f32,
    /// The wait after calling someone over, and after finding nobody.
    pub after_use: f32,
    pub retry: f32,
}

impl UseTimer {
    /// The timer of an object of `record` coming in, `r` a random number
    /// in [0, 1).
    pub fn new(record: &SocialUse, r: f32) -> Self {
        Self {
            left: r * record.rest,
            after_use: record.rest,
            retry: RETRY,
        }
    }

    /// Counts `dt` seconds. When the time runs out `recruit` is tried,
    /// and the timer starts again by whether it took someone, `r` a
    /// random number in [0, 1). True if someone was taken.
    pub fn step(&mut self, dt: f32, r: f32, recruit: impl FnOnce() -> bool) -> bool {
        if dt <= 0.0 {
            return false;
        }
        self.left -= dt;
        if self.left > 0.0 {
            return false;
        }
        let taken = recruit();
        self.left = r * JITTER + if taken { self.after_use } else { self.retry };
        taken
    }
}

/// The first of `candidates` (who they are, where) under `range` metres
/// from the object at `at`: taken off the list and returned.
pub fn recruit<T>(at: Vec3, range: f32, candidates: &mut Vec<(T, Vec3)>) -> Option<T> {
    let i = candidates
        .iter()
        .position(|(_, p)| p.distance_squared(at) < range * range)?;
    Some(candidates.remove(i).0)
}

/// Whether a person idling this frame with the use meter `meter` joins
/// a list of `listed` candidates.
pub fn is_candidate(meter: f32, listed: usize) -> bool {
    meter < CANDIDATE_BELOW && listed < CANDIDATES_MOST
}

/// A person's use meter: how lately, and for how long, they have used
/// something.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UseMeter(pub f32);

impl UseMeter {
    /// One think of `dt` seconds, using a thing whose record's duration
    /// is `using`, or nothing.
    pub fn step(&mut self, dt: f32, using: Option<f32>) {
        match using {
            Some(duration) if self.0 < METER_MOST => {
                let rate = if duration > 1e-4 {
                    METER_DONE / duration
                } else {
                    METER_FAST
                };
                self.0 += rate * dt;
            }
            _ if self.0 > 0.0 => self.0 -= METER_DRAIN * dt,
            _ => self.0 = 0.0,
        }
    }

    /// Whether the use is over.
    pub fn done(self) -> bool {
        self.0 > METER_DONE
    }
}

/// How long a `LivingWorldObject` civilian stays at a thing of
/// `duration` (none, or not over 0: [`STAY_DEFAULT`]), `r` a random
/// number in [0, 1).
pub fn stay_seconds(duration: Option<f32>, r: f32) -> f32 {
    let base = duration.filter(|&d| d > 0.0).unwrap_or(STAY_DEFAULT);
    (r * 0.2 + 0.9) * base
}

#[cfg(test)]
mod tests {
    use super::*;

    const BENCH: SocialUse = SocialUse {
        kind: 2,
        range: 8.0,
        rest: 60.0,
        duration: 30.0,
    };

    #[test]
    fn the_unnamed_fields_are_not_the_named_ones() {
        assert_ne!(KIND, name_hash("Type"));
        assert_ne!(REST, name_hash("Range"));
        assert_ne!(REST, name_hash("duration"));
    }

    #[test]
    fn a_timer_starts_at_a_share_of_the_rest_and_restarts_by_the_outcome() {
        let mut t = UseTimer::new(&BENCH, 0.5);
        assert_eq!((t.left, t.after_use, t.retry), (30.0, 60.0, 5.0));
        // Not yet out: nobody is asked.
        assert!(!t.step(29.0, 0.0, || panic!("too soon")));
        // Out, and nobody near: again in 5 s and a little.
        assert!(!t.step(1.5, 1.0, || false));
        assert!((t.left - 5.15).abs() < 1e-5);
        // Out, and someone taken: the record's wait.
        assert!(t.step(6.0, 0.0, || true));
        assert_eq!(t.left, 60.0);
        // No time passing, nothing happens.
        t.left = 0.0;
        assert!(!t.step(0.0, 0.0, || panic!("no time passed")));
    }

    #[test]
    fn the_first_candidate_in_range_is_taken_off_the_list() {
        let at = Vec3::new(10.0, 0.0, 10.0);
        let mut list = vec![
            (1, Vec3::new(30.0, 0.0, 10.0)),
            (2, Vec3::new(14.0, 0.0, 10.0)),
            (3, Vec3::new(11.0, 0.0, 10.0)),
        ];
        // Not the nearest: the first in the list within the range.
        assert_eq!(recruit(at, BENCH.range, &mut list), Some(2));
        assert_eq!(list.iter().map(|c| c.0).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(recruit(at, BENCH.range, &mut list), Some(3));
        assert_eq!(recruit(at, BENCH.range, &mut list), None);
        // The range itself is out; height counts.
        let mut edge = vec![(4, Vec3::new(18.0, 0.0, 10.0))];
        assert_eq!(recruit(at, BENCH.range, &mut edge), None);
        let mut above = vec![(5, Vec3::new(10.0, 9.0, 10.0))];
        assert_eq!(recruit(at, BENCH.range, &mut above), None);
        // A range of 0 (`TiredBench`) calls nobody over.
        let mut on_it = vec![(6, at)];
        assert_eq!(recruit(at, 0.0, &mut on_it), None);
    }

    #[test]
    fn a_use_lasts_its_duration_and_keeps_them_off_the_list_a_while() {
        let mut m = UseMeter::default();
        assert!(is_candidate(m.0, 0));
        assert!(!is_candidate(m.0, CANDIDATES_MOST));
        // On a bench: over 10 just after its 30 s.
        let mut seconds = 0.0;
        while !m.done() {
            m.step(0.5, Some(BENCH.duration));
            seconds += 0.5;
        }
        assert!((30.0..=31.0).contains(&seconds), "{seconds}");
        assert!(!is_candidate(m.0, 0));
        // Up again, it drains a tenth a second: under 5 after 50 s more.
        let mut after = 0.0;
        while !is_candidate(m.0, 0) {
            m.step(0.5, None);
            after += 0.5;
        }
        assert!((50.0..=56.0).contains(&after), "{after}");
        // It stops rising at 15, and never falls under 0.
        let mut high = UseMeter(METER_MOST);
        high.step(1.0, Some(1.0));
        assert!((high.0 - (METER_MOST - METER_DRAIN)).abs() < 1e-5);
        let mut low = UseMeter(0.0);
        low.step(1.0, None);
        assert_eq!(low.0, 0.0);
        // A use of no duration is over in half a second and a little.
        let mut quick = UseMeter::default();
        quick.step(0.6, Some(0.0));
        assert!(quick.done());
    }

    #[test]
    fn a_lighter_civilian_stays_the_duration_give_or_take_a_tenth() {
        assert!((stay_seconds(Some(30.0), 0.0) - 27.0).abs() < 1e-4);
        assert!((stay_seconds(Some(30.0), 0.5) - 30.0).abs() < 1e-4);
        assert!(stay_seconds(Some(30.0), 0.999) < 33.0);
        assert!((stay_seconds(None, 0.5) - 5.0).abs() < 1e-5);
        assert!((stay_seconds(Some(0.0), 0.5) - 5.0).abs() < 1e-5);
    }
}
