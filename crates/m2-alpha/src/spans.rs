//! Where play's time goes, for the `pace:` lines: parts of a frame are
//! timed by name ([`time`]) and summed until taken ([`take`]). Playtest 7
//! ran at 1 fps and its log could say only that, not why.
//!
//! Spans may nest (the game's tick holds its props' update): each sums
//! its own time, so the parts don't add up to the whole.

use std::cell::RefCell;
use std::time::{Duration, Instant};

thread_local! {
    static SPENT: RefCell<Vec<(&'static str, Duration)>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f`, adding the time it took to `name`'s.
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let out = f();
    add(name, started.elapsed());
    out
}

/// Adds `took` to `name`'s time.
pub fn add(name: &'static str, took: Duration) {
    SPENT.with_borrow_mut(|spent| match spent.iter_mut().find(|(n, _)| *n == name) {
        Some((_, t)) => *t += took,
        None => spent.push((name, took)),
    });
}

/// Times a run of code in sections: each [`mark`](Self::mark) adds the
/// time since the last (or since [`new`](Self::new)) to its name's.
#[derive(Debug)]
pub struct Lap(Instant);

impl Lap {
    pub fn new() -> Self {
        Self(Instant::now())
    }

    /// Ends the section `name` and starts the next.
    pub fn mark(&mut self, name: &'static str) {
        let now = Instant::now();
        add(name, now - self.0);
        self.0 = now;
    }
}

impl Default for Lap {
    fn default() -> Self {
        Self::new()
    }
}

/// The times summed since last taken, longest first.
pub fn take() -> Vec<(&'static str, Duration)> {
    let mut spent = SPENT.with_borrow_mut(std::mem::take);
    spent.sort_by_key(|s| std::cmp::Reverse(s.1));
    spent
}

/// `spent` over `span` seconds, as a log line's tail: each part's
/// milliseconds a second, the longest `most`.
pub fn describe(spent: &[(&'static str, Duration)], span: f32, most: usize) -> String {
    spent
        .iter()
        .take(most)
        .map(|(n, t)| format!("{n} {:.0}", t.as_secs_f32() * 1000.0 / span.max(1e-3)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_sum_by_name_longest_first() {
        let _ = take();
        add("a", Duration::from_millis(10));
        add("b", Duration::from_millis(30));
        add("a", Duration::from_millis(5));
        let spent = take();
        assert_eq!(
            spent,
            [
                ("b", Duration::from_millis(30)),
                ("a", Duration::from_millis(15))
            ]
        );
        assert!(take().is_empty());
        assert_eq!(describe(&spent, 2.0, 1), "b 15");
    }
}
