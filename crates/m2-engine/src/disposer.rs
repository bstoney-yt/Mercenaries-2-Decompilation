//! The disposer: what the 360 keeps of the objects a blast threw about,
//! and which it takes away once there are too many.
//!
//! **From the 360 code** (`PgDebrisManager`, set up by 0x822085A8):
//!
//! - Five pools of objects, each holding at most 40, 64, 8, 40 and 20.
//!   They take at most 16 (the first pool) and 32 (the second) new ones
//!   between two updates (+0xDD8, counted by +0xDDC and reset by the
//!   update 0x822466E0).
//! - Every object a blast pushes goes into the first pool (0x82289F40,
//!   by 0x82246A08), when the disposer is on (bit 0x40 of its settings).
//!   The scripts' `Object.AddToDisposer` puts an object into one of the
//!   first three pools by the kind of thing it is (0x822572C0).
//! - Each entry has a weight (0x822470E0, [`priority`]). Half of it is
//!   how young the entry is: 1 when it comes in, down to 0 over 5 s at
//!   0.2 a second. The other half is how big the object looks from the
//!   nearest camera that sees it (0x82246C08, [`seen_size`]): its
//!   radius over its distance, from 0.005 (nothing) to 0.2 (all).
//! - The pool keeps its entries lightest first (0x82247230). A new one
//!   that comes into a full pool takes the place of the lightest, which
//!   is disposed of (0x822480A8: the object is removed). If the new one
//!   is lighter than the lightest, it is not taken in.
//! - Each update weighs every entry again and drops those whose object
//!   has gone (0x822466E0).
//!
//! **Not ported:** a disposed object still in view is handed to a queue
//! (0x822567D0) instead of being removed at once; here it is removed.
//! The pools' other per-update rules (+0xDD0 -> 0x822475A8, +0xDD1 ->
//! 0x82247EC0, +0xDD2, +0xDD3 -> 0x82247870, +0xDD4 -> 0x82247A28) are
//! unread. The camera's facing (an object behind the camera, more than
//! its radius plus a little off, counts as unseen) is not ported: the one
//! eye given sees all round.

/// How many objects a blast threw the first pool keeps (0x822085A8).
pub const BLAST_CAP: usize = 40;
/// How many it takes in between two updates (+0xDD8 = 0x10).
pub const BLAST_PER_UPDATE: usize = 16;

/// Seconds over which an entry's youth goes from 1 to 0 (0x820B8054 =
/// 0.2 a second).
const AGE_RATE: f32 = 0.2;
/// The radius over distance below which an object counts as unseen
/// (0x820F8C6C), and at which it counts as wholly seen (0x820B8054).
const SEEN_LEAST: f32 = 0.005;
const SEEN_MOST: f32 = 0.2;
/// The least a seen object's size counts (0x820B7A44).
const SEEN_FLOOR: f32 = 1.0e-4;

/// How big a sphere of `radius` looks from `distance` off (0x82246C08):
/// 0 when its radius over its distance is under 0.005, 1 from 0.2.
pub fn seen_size(radius: f32, distance: f32) -> f32 {
    let d2 = distance * distance;
    let r = if d2 < SEEN_FLOOR {
        f32::MAX
    } else {
        (radius * radius / d2).sqrt()
    };
    if r <= 0.0 {
        return 0.0;
    }
    ((r - SEEN_LEAST) / (SEEN_MOST - SEEN_LEAST)).clamp(SEEN_FLOOR, 1.0)
}

/// An entry's weight (0x822470E0): half how young it is (`age` seconds
/// since it came in; a negative age counts as old) and half how big it
/// looks ([`seen_size`]).
pub fn priority(age: f32, seen: f32) -> f32 {
    let age = if age < 0.0 { 5.0 } else { age };
    let old = (age * AGE_RATE).clamp(0.0, 1.0);
    ((1.0 - old) * 0.5 + seen * 0.5).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    key: u32,
    weight: f32,
    added: f32,
}

/// What became of an object put in ([`Disposer::admit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    /// Taken in.
    Taken,
    /// Taken in, and this one, the lightest, disposed of to make room.
    Disposing(u32),
    /// Not taken in: lighter than every entry of a full pool, or more
    /// than it takes between two updates. It stays, untracked.
    Refused,
}

/// One pool.
#[derive(Clone, Debug)]
pub struct Disposer {
    cap: usize,
    per_update: usize,
    taken: usize,
    /// Lightest first.
    entries: Vec<Entry>,
}

impl Default for Disposer {
    /// The pool what a blast throws goes into.
    fn default() -> Self {
        Self::new(BLAST_CAP, BLAST_PER_UPDATE)
    }
}

impl Disposer {
    pub fn new(cap: usize, per_update: usize) -> Self {
        Self {
            cap,
            per_update,
            taken: 0,
            entries: Vec::new(),
        }
    }

    /// How many it holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// What it holds, lightest first.
    pub fn keys(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.iter().map(|e| e.key)
    }

    /// Whether it holds `key`.
    pub fn holds(&self, key: u32) -> bool {
        self.entries.iter().any(|e| e.key == key)
    }

    /// Object `key` comes in at `now` (seconds), looking `seen` big
    /// ([`seen_size`]).
    pub fn admit(&mut self, key: u32, now: f32, seen: f32) -> Admit {
        if self.holds(key) {
            return Admit::Taken;
        }
        if self.taken >= self.per_update {
            return Admit::Refused;
        }
        self.taken += 1;
        let new = Entry {
            key,
            weight: priority(0.0, seen),
            added: now,
        };
        let mut out = Admit::Taken;
        if self.entries.len() >= self.cap {
            if new.weight < self.entries[0].weight {
                return Admit::Refused;
            }
            out = Admit::Disposing(self.entries.remove(0).key);
        }
        let at = self.entries.partition_point(|e| e.weight <= new.weight);
        self.entries.insert(at, new);
        out
    }

    /// Weighs every entry again at `now`: `seen` says how big each looks
    /// ([`seen_size`]), `None` for one whose object has gone, which is
    /// dropped. Then it takes new ones again.
    pub fn update(&mut self, now: f32, seen: impl Fn(u32) -> Option<f32>) {
        self.entries.retain_mut(|e| match seen(e.key) {
            Some(s) => {
                e.weight = priority(now - e.added, s);
                true
            }
            None => false,
        });
        self.entries.sort_by(|a, b| a.weight.total_cmp(&b.weight));
        self.taken = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn how_big_it_looks_runs_from_a_two_hundredth_to_a_fifth() {
        // A 1 m object 300 m off is unseen (as good as).
        assert_eq!(seen_size(1.0, 300.0), SEEN_FLOOR);
        // 5 m off: 0.2, all of it.
        assert_eq!(seen_size(1.0, 5.0), 1.0);
        // 10 m off: 0.1, about half.
        assert!((seen_size(1.0, 10.0) - 0.4872).abs() < 1e-3);
        // On top of the eye.
        assert_eq!(seen_size(1.0, 0.0), 1.0);
    }

    #[test]
    fn its_weight_is_half_youth_and_half_size() {
        assert_eq!(priority(0.0, 1.0), 1.0);
        assert_eq!(priority(0.0, 0.0), 0.5);
        assert_eq!(priority(2.5, 0.0), 0.25);
        assert_eq!(priority(5.0, 0.0), 0.0);
        assert_eq!(priority(9.0, 0.5), 0.25);
        // A negative age counts as old.
        assert_eq!(priority(-1.0, 0.0), 0.0);
    }

    #[test]
    fn a_full_pool_disposes_of_the_lightest_for_a_heavier_one() {
        let mut d = Disposer::new(2, 16);
        assert_eq!(d.admit(1, 0.0, 0.0), Admit::Taken);
        assert_eq!(d.admit(2, 0.0, 0.5), Admit::Taken);
        // Six seconds on, both old: 1 weighs 0, 2 a quarter.
        d.update(6.0, |k| Some(if k == 2 { 0.5 } else { 0.0 }));
        assert_eq!(d.admit(3, 6.0, 0.0), Admit::Disposing(1));
        assert!(d.holds(2) && d.holds(3) && !d.holds(1));
        // A new one far off still outweighs one old and far off.
        d.update(20.0, |_| Some(0.0));
        assert_eq!(d.admit(4, 20.0, 0.0), Admit::Disposing(2));
    }

    #[test]
    fn a_full_pool_refuses_one_lighter_than_all_it_holds() {
        let mut d = Disposer::new(1, 16);
        assert_eq!(d.admit(1, 0.0, 1.0), Admit::Taken);
        // 1 weighs 1; a new one unseen weighs half.
        assert_eq!(d.admit(2, 0.0, 0.0), Admit::Refused);
        assert!(d.holds(1));
    }

    #[test]
    fn it_takes_only_so_many_between_updates_and_drops_the_gone() {
        let mut d = Disposer::new(40, 16);
        for k in 0..20 {
            let a = d.admit(k, 0.0, 0.0);
            assert_eq!(a == Admit::Taken, k < 16, "{k}");
        }
        d.update(0.1, |k| (k % 2 == 0).then_some(0.0));
        assert_eq!(d.len(), 8);
        assert_eq!(d.admit(30, 0.1, 0.0), Admit::Taken);
    }
}
