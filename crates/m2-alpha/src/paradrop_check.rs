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
//! physics in the data), so the men drop where it was put.

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
    done: bool,
}

impl Check {
    /// One step of `dt` seconds of play.
    pub fn tick(&mut self, game: &Game, mission: &mut Mission, dt: f32) {
        if self.done {
            return;
        }
        self.played += dt;
        let Some(her) = game.scene.player.as_ref().map(|p| p.character.position) else {
            return;
        };
        if self.plane.is_none() {
            if self.played >= AFTER {
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
            return;
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
            if let Some(e) = n.spawn
                && n.dropped
                && self.jumpers.contains(&e)
                && self.fell.insert(e)
            {
                eprintln!(
                    "paradrop-check: jumper {e:#x} in play, falling at {:.2} m/s2 from {:.0}",
                    n.character.gravity, n.character.position
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
            "{} dropped, {} fell, {removed} removed, {} soldiers",
            self.jumpers.len(),
            self.fell.len(),
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
    }
}
