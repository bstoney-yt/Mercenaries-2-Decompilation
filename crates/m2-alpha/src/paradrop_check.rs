//! `--paradrop-check`: with `--mission`, the Allied paradrop plane
//! (`Support Vehicle (Paradrop_AL)`, its script `paradrop`) is put 100 m
//! over her once the scripts have run a few seconds, and what follows is
//! watched through the scripts' world and the people in play: the
//! paratroopers it drops (`Allied Paratrooper`), each falling slowly
//! under his template's gravity scale, losing his chute as his landing
//! hurts him (`paratrooper`'s health watch) and leaving an `Allied
//! Airborne` soldier in his place. One line each step, then
//!
//! ```text
//! paradrop-check: PASS <n> dropped, <n> fell, <n> removed, <n> soldiers
//! paradrop-check: FAIL <what fell short>: ...
//! ```
//!
//! Needs no contract. The plane is the scripts' only (it has no model or
//! physics in the data), so the men drop where it was put: over dry
//! ground, so she is first put ashore if she is at sea (the first
//! contract starts her in the water 658 m off the beach; men dropped into
//! the sea swim and never shed their chutes, as no landing hurts them).

use std::collections::HashSet;

use m2_game::mission::Mission;

use crate::game::Game;

/// The plane put over her.
pub const PLANE: &str = "Support Vehicle (Paradrop_AL)";
/// The men it drops.
pub const JUMPER: &str = "Allied Paratrooper";
/// The soldier each becomes.
pub const SOLDIER: &str = "Allied Airborne";
/// How many the plane's script drops (`paradrop`: 16 timers).
pub const DROPS: usize = 16;
/// Metres over her the plane is put (the paradrop locations fly theirs
/// 100 m over themselves).
const OVER: f32 = 100.0;
/// Seconds of play before the plane is put up.
const AFTER: f32 = 5.0;
/// Seconds she stands ashore before it is.
const SETTLE: f32 = 2.0;
/// Seconds she has to come ashore before the check gives up.
const ASHORE_WAIT: f32 = 60.0;
/// Seconds after it the check gives up: the last man leaves at 17.25 s
/// and falls 100 m at 1.96 m/s² in about 10 s.
const GIVE_UP: f32 = 90.0;

/// The check's progress.
#[derive(Debug, Default)]
pub struct Check {
    played: f32,
    plane: Option<u32>,
    since: f32,
    jumpers: HashSet<u32>,
    soldiers: HashSet<u32>,
    /// Jumpers seen falling in play.
    fell: HashSet<u32>,
    /// Jumpers that came down in water.
    wet: HashSet<u32>,
    /// Seconds she has stood on dry ground.
    ashore_for: f32,
    /// Seconds to the next ask to put her ashore (each costs many rays).
    ask_in: f32,
    done: bool,
}

/// What the check asks of play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Nothing,
    /// Put her on the nearest dry ground.
    Ashore,
}

impl Check {
    /// One step of `dt` seconds of play.
    pub fn tick(&mut self, game: &Game, mission: &mut Mission, dt: f32) -> Ask {
        if self.done {
            return Ask::Nothing;
        }
        self.played += dt;
        let Some(player) = game.scene.player.as_ref() else {
            return Ask::Nothing;
        };
        let her = player.character.position;
        let water = |p: m2_math::glam::Vec3| {
            game.scene
                .world
                .as_ref()
                .and_then(|w| w.physics.water_at(p.x, p.z))
                .is_some_and(|w| p.y < w + 0.5)
        };
        if self.plane.is_none() {
            if self.played < AFTER {
                return Ask::Nothing;
            }
            // Over dry ground only: at sea she is put ashore first.
            let dry = player.landed && !water(her);
            self.ashore_for = if dry { self.ashore_for + dt } else { 0.0 };
            if !dry {
                if self.played >= AFTER + ASHORE_WAIT {
                    eprintln!(
                        "paradrop-check: FAIL she never stood on dry ground (at {her:.0} after {:.0} s)",
                        self.played
                    );
                    self.done = true;
                    return Ask::Nothing;
                }
                self.ask_in -= dt;
                if self.ask_in > 0.0 {
                    return Ask::Nothing;
                }
                self.ask_in = 0.5;
                return Ask::Ashore;
            }
            if self.ashore_for >= SETTLE {
                let at = [her.x, her.y + OVER, her.z];
                self.plane = mission.spawn(PLANE, at);
                match self.plane {
                    Some(g) => eprintln!("paradrop-check: {PLANE} put up as {g:#x} at {at:.0?}"),
                    None => {
                        eprintln!("paradrop-check: FAIL the plane {PLANE} did not spawn");
                        self.done = true;
                    }
                }
            }
            return Ask::Nothing;
        }
        self.since += dt;
        {
            let w = mission.world().borrow();
            for (&g, o) in &w.objects {
                if o.name.eq_ignore_ascii_case(JUMPER) && self.jumpers.insert(g) {
                    eprintln!(
                        "paradrop-check: jumper {g:#x} dropped at {:.0?}",
                        o.position
                    );
                } else if o.name.eq_ignore_ascii_case(SOLDIER) && self.soldiers.insert(g) {
                    eprintln!(
                        "paradrop-check: soldier {g:#x} put in at {:.0?}",
                        o.position
                    );
                }
            }
        }
        for n in &game.scene.people.npcs {
            let Some(e) = n.spawn.filter(|e| self.jumpers.contains(e)) else {
                continue;
            };
            if n.dropped && self.fell.insert(e) {
                eprintln!(
                    "paradrop-check: jumper {e:#x} in play, falling at {:.2} m/s2 from {:.0}",
                    n.character.gravity, n.character.position
                );
            }
            if n.landed && water(n.character.position) && self.wet.insert(e) {
                eprintln!(
                    "paradrop-check: jumper {e:#x} came down in water at {:.0}: no landing hurts him",
                    n.character.position
                );
            }
        }
        let removed = {
            let w = mission.world().borrow();
            self.jumpers
                .iter()
                .filter(|g| !w.objects.contains_key(g))
                .count()
        };
        let summary = format!(
            "{} dropped, {} fell, {} in water, {removed} removed, {} soldiers",
            self.jumpers.len(),
            self.fell.len(),
            self.wet.len(),
            self.soldiers.len()
        );
        if self.jumpers.len() >= DROPS
            && removed >= DROPS
            && self.soldiers.len() >= DROPS
            && self.fell.len() >= DROPS
        {
            eprintln!("paradrop-check: PASS {summary} in {:.1} s", self.since);
            self.done = true;
        } else if self.since >= GIVE_UP {
            let short = if self.jumpers.len() < DROPS {
                "too few dropped"
            } else if self.fell.len() < DROPS {
                "too few fell in play"
            } else if removed < DROPS {
                "too few shed their chutes"
            } else {
                "too few soldiers"
            };
            eprintln!("paradrop-check: FAIL {short}: {summary} after {GIVE_UP} s");
            self.done = true;
        }
        Ask::Nothing
    }
}
