//! The nightly autoplay runner: plays every contract of the user's own
//! install with `m2-alpha --mission --autoplay` on the dev VM, one at a
//! time inside an off-hours window, and flags what broke (panics, crashes,
//! hangs, stuck objectives, falls through the world, frame-time cliffs,
//! contracts not completed). Each run writes `result.json` and
//! `summary.txt`; the night writes `night.json` and a `report.md` under
//! 3 KB; `issues` turns the breaks into one GitHub issue each.
//!
//! See `docs/nightly-autoplay.md` for how it is run and scheduled.
//!
//! Provenance: the contract list is read from the install's `vz.wad`
//! index (from samples); every detector threshold is a tooling heuristic
//! (invented). Nothing here encodes a game rule.

pub mod contracts;
pub mod detect;
pub mod gpu;
pub mod issues;
pub mod night;
pub mod run;

#[cfg(test)]
mod tests;
