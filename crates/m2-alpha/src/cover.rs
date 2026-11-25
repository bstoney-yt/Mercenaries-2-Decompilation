//! The cover over a new game's start: the loading screen goes up on the
//! first frame after New Game is accepted and stays until the island
//! around her has loaded, she has been put at her start and stands, and
//! the pipelines listed for building ahead are built; only then does play
//! (the scripts' intro movie, `01_AOA_J`) show. Before this the world was
//! drawn from the first frame after New Game, with her where the scene
//! was built, until the scripts' far move raised the arrival screen
//! ([`crate::game::FAR_TELEPORT`]), whose wipe in let the world show
//! through for another 2 s.
//!
//! [`Cover`] is the state machine ([`State`]: covered, fading out, clear),
//! owned by the flow ([`crate::flow::Flow::cover`]) and ticked with the
//! game's ticks; while it covers, the frame draws black and the loading
//! screen, never the world.
//!
//! # Provenance
//!
//! - From the 360 code: a single-player start (`Sys.StartSingleplayer`,
//!   `0x8220D848`; a save's load, `0x82464830`, calls it too) pushes the
//!   start state, whose enter (`0x82211AC0`) puts up the
//!   `loadingscreen_standalone` movie (`0x82370208`) and whose update
//!   (`0x82211C48`) pushes Loading (`0x96FB0F27`). Loading's enter
//!   (`0x8220FB70`) raises the GUI's loading flag (`0x82370180(gui, 1, 2)`)
//!   and holds every live player (`0x8243A7E0(player, 1)`: a hold object
//!   at player `+0x190`). Loading's update (`0x8220FEC8`) counts ticks in a
//!   row with nothing changed (no player's streaming focus moved, the
//!   pending-object queue `0x835A7A10` empty, the flag `0x82DC2EF4` clear,
//!   and the resource manager's two queue counts, `+0x814` and `+0x824`,
//!   both 0); any other tick sets the count back to 0. Once it reaches 10
//!   ([`QUIET_TICKS`]) it lets the players go (`0x8243A7E0(player, 0)`)
//!   and pushes WaitForStreaming (`0x7E289119`), whose update
//!   (`0x8220F8B8`) counts its ticks and its time and pushes InGame
//!   (`0x57B5E35A`) once the count passes 10 ([`STREAM_TICKS`]) or the
//!   time passes `*0x82B7CBA4` = 30.0 s ([`STREAM_CAP`]). So the players
//!   are let go, and fall and settle, for those last ticks while the
//!   screen is still up.
//! - From samples: the loading movie, its wipe back (`fadeOut`) when it
//!   goes ([`crate::loading`]).
//! - Port-side mapping (inferred): the 360's "nothing changed" is, here,
//!   the island's tiles all in, no vehicle, prop or world piece still
//!   loading near her ([`crate::game::Game::streaming_busy`]), the
//!   scripts running and her standing; a move of hers by the scripts
//!   ([`crate::game::Game::placements`]) is a change (the 360's moved
//!   streaming focus), and starts Loading's count again.
//! - Invented: waiting for the listed pipelines to be built
//!   ([`crate::warmup::pending`]) before the cover drops; waiting up to
//!   [`PLACE_WAIT`] for the scripts to put her at her start (then she is
//!   taken as placed where she stands); holding the scripts' movies while
//!   covered, and waiting up to [`INTRO_WAIT`] for theirs once ready
//!   (none: the loading screen wipes back to play); the [`FADE`] from
//!   black into the intro movie; the [`CAP`] past which the cover gives
//!   up waiting for her or the pipelines.

/// Loading's ticks in a row with nothing changed before it lets the
/// players go (from the 360 code: `0x8220FEC8`, `9 < count`).
pub const QUIET_TICKS: u32 = 10;
/// WaitForStreaming's ticks: InGame comes on the tick its count passes
/// this (from the 360 code: `0x8220F8B8`, `10 < count`).
pub const STREAM_TICKS: u32 = 10;
/// WaitForStreaming's time limit, seconds (from the 360 code:
/// `*0x82B7CBA4` = 30.0).
pub const STREAM_CAP: f32 = 30.0;
/// Seconds after the scripts start that she counts as placed where she
/// stands if they never move her (invented).
pub const PLACE_WAIT: f32 = 10.0;
/// Seconds the ready cover waits for the scripts' intro movie
/// (invented).
pub const INTRO_WAIT: f32 = 5.0;
/// Seconds of the fade from black into the intro movie (invented).
pub const FADE: f32 = 0.5;
/// Seconds after which the cover stops waiting for her and the pipelines
/// (invented; as [`crate::loading::LOADING_CAP`]).
pub const CAP: f32 = 90.0;

/// The 360 state the cover is in, or the port's own waits after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Loading: `quiet` ticks in a row with nothing changed.
    Loading { quiet: u32 },
    /// WaitForStreaming: its ticks and seconds.
    Streaming { ticks: u32, secs: f32 },
    /// InGame reached: waiting for her to stand and the pipelines to be
    /// built (invented).
    Settling,
    /// Ready: the scripts' intro movie may come (invented wait).
    Intro { waited: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// The loading screen (over black) is all that shows.
    Covered(Phase),
    /// Fading from black into the intro movie: `left` seconds.
    FadingOut { left: f32 },
    /// Play shows.
    Clear,
}

/// What the cover looks at each tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    /// Something near her (or the island's tiles) is still loading.
    pub busy: bool,
    /// The scripts run.
    pub scripts: bool,
    /// She stands (on the ground or in a seat), not falling.
    pub standing: bool,
    /// How many times the scripts (or the start) have put her somewhere.
    pub placements: u32,
    /// Pipelines listed for building ahead not yet built.
    pub pipelines_left: usize,
}

#[derive(Clone, Debug)]
pub struct Cover {
    pub state: State,
    /// Seconds since it went up.
    pub age: f32,
    /// Seconds the scripts have run.
    scripts_for: f32,
    /// Placements seen when it went up, and last tick.
    placements: Option<(u32, u32)>,
    /// She was placed since it went up (or taken as placed).
    pub placed: bool,
}

impl Default for Cover {
    fn default() -> Self {
        Self::clear()
    }
}

impl Cover {
    /// Up: New Game (or a save) was just accepted.
    pub fn raise() -> Self {
        Self {
            state: State::Covered(Phase::Loading { quiet: 0 }),
            age: 0.0,
            scripts_for: 0.0,
            placements: None,
            placed: false,
        }
    }

    /// Down: nothing covers.
    pub fn clear() -> Self {
        Self {
            state: State::Clear,
            ..Self::raise()
        }
    }

    /// Whether only black and the loading screen show.
    pub fn covered(&self) -> bool {
        matches!(self.state, State::Covered(_))
    }

    /// How dark the cover is over what shows, 0 to 1.
    pub fn black(&self) -> f32 {
        match self.state {
            State::Covered(_) => 1.0,
            State::FadingOut { left } => (left / FADE).clamp(0.0, 1.0),
            State::Clear => 0.0,
        }
    }

    /// Whether the scripts' movies wait: while covered and not yet ready.
    pub fn holds_movies(&self) -> bool {
        matches!(self.state, State::Covered(p) if !matches!(p, Phase::Intro { .. }))
    }

    /// The scripts' movie (the intro) started: the cover fades into it.
    pub fn movie_started(&mut self) -> Option<String> {
        if !self.covered() {
            return None;
        }
        self.state = State::FadingOut { left: FADE };
        Some(format!("cover: fading into the intro at {:.1} s", self.age))
    }

    /// Moves the fade on by a frame's `dt` (the movie plays; the game
    /// waits).
    pub fn frame(&mut self, dt: f32) {
        if let State::FadingOut { left } = &mut self.state {
            *left -= dt;
            if *left <= 0.0 {
                self.state = State::Clear;
            }
        }
    }

    /// One game tick of `dt` seconds: a log line when the state changes.
    pub fn tick(&mut self, i: Inputs, dt: f32) -> Option<String> {
        let State::Covered(phase) = self.state else {
            return None;
        };
        self.age += dt;
        if i.scripts {
            self.scripts_for += dt;
        }
        let (first, last) = *self.placements.get_or_insert((i.placements, i.placements));
        let moved = i.placements != last;
        self.placements = Some((first, i.placements));
        if i.placements != first || (!self.placed && i.standing && self.scripts_for >= PLACE_WAIT) {
            self.placed = true;
        }
        let capped = self.age >= CAP;
        let next = match phase {
            // A move of hers starts Loading over, wherever it was.
            _ if moved => Phase::Loading { quiet: 0 },
            Phase::Loading { quiet } => {
                let still = !i.busy && i.scripts && i.standing && self.placed;
                let quiet = if still { quiet + 1 } else { 0 };
                if quiet >= QUIET_TICKS || capped {
                    Phase::Streaming {
                        ticks: 0,
                        secs: 0.0,
                    }
                } else {
                    Phase::Loading { quiet }
                }
            }
            Phase::Streaming { ticks, secs } => {
                let (ticks, secs) = (ticks + 1, secs + dt);
                if ticks > STREAM_TICKS || secs > STREAM_CAP {
                    Phase::Settling
                } else {
                    Phase::Streaming { ticks, secs }
                }
            }
            Phase::Settling => {
                if (i.standing && self.placed && i.pipelines_left == 0) || capped {
                    Phase::Intro { waited: 0.0 }
                } else {
                    Phase::Settling
                }
            }
            Phase::Intro { waited } => Phase::Intro {
                waited: waited + dt,
            },
        };
        if let Phase::Intro { waited } = next
            && waited >= INTRO_WAIT
        {
            self.state = State::Clear;
            return Some(format!(
                "cover: no intro came: down at {:.1} s (the loading screen wipes back)",
                self.age
            ));
        }
        self.state = State::Covered(next);
        let changed = std::mem::discriminant(&phase) != std::mem::discriminant(&next);
        changed.then(|| {
            format!(
                "cover: {} at {:.1} s{}",
                match next {
                    Phase::Loading { .. } => "Loading again (she was moved)",
                    Phase::Streaming { .. } => "WaitForStreaming",
                    Phase::Settling => "InGame: waiting for her to stand and the pipelines",
                    Phase::Intro { .. } => "ready: the intro may play",
                },
                self.age,
                if capped { " (capped)" } else { "" }
            )
        })
    }

    /// A short name of the state, for the check's log.
    pub fn name(&self) -> &'static str {
        match self.state {
            State::Covered(Phase::Loading { .. }) => "Covered/Loading",
            State::Covered(Phase::Streaming { .. }) => "Covered/WaitForStreaming",
            State::Covered(Phase::Settling) => "Covered/Settling",
            State::Covered(Phase::Intro { .. }) => "Covered/Intro",
            State::FadingOut { .. } => "FadingOut",
            State::Clear => "Clear",
        }
    }
}

/// `--newgame-cover-check`: what each frame from New Game to the intro
/// showed.
#[derive(Clone, Debug, Default)]
pub struct Check {
    /// Frames since New Game.
    pub frames: u64,
    /// The first frame that showed the world with nothing over it.
    pub first_bad: Option<u64>,
    /// When the cover first stopped covering: whether she had been
    /// placed and stood.
    pub dropped: Option<(u64, bool)>,
    /// The frame the intro movie started on.
    pub intro: Option<u64>,
}

impl Check {
    /// A frame: `world` whether the world was drawn, `visible` whether it
    /// showed (nothing opaque over it), `placed` and `standing` hers.
    /// Returns its log line.
    pub fn frame(
        &mut self,
        cover: &Cover,
        world: bool,
        visible: bool,
        placed: bool,
        standing: bool,
    ) -> String {
        let n = self.frames;
        self.frames += 1;
        if self.intro.is_none() && visible && self.first_bad.is_none() {
            self.first_bad = Some(n);
        }
        if self.dropped.is_none() && !cover.covered() {
            self.dropped = Some((n, placed && standing));
        }
        format!(
            "newgame-cover: frame {n} {} world pass {} visible {} placed {placed} standing {standing}",
            cover.name(),
            if world { "ran" } else { "skipped" },
            visible
        )
    }

    /// The intro movie started on this frame.
    pub fn intro(&mut self) {
        self.intro.get_or_insert(self.frames);
    }

    /// PASS only if no frame before the intro showed the world and she
    /// was placed and standing when the cover dropped.
    pub fn verdict(&self) -> (bool, String) {
        if let Some(n) = self.first_bad {
            return (
                false,
                format!("FAIL frame {n} showed the world uncovered before the intro"),
            );
        }
        match (self.intro, self.dropped) {
            (None, _) => (false, format!("FAIL no intro after {} frames", self.frames)),
            (Some(_), Some((d, false))) => (
                false,
                format!("FAIL the cover dropped at frame {d} before she was placed and standing"),
            ),
            (Some(i), Some((d, true))) => (
                true,
                format!(
                    "PASS no world frame before the intro (frame {i}); cover dropped at frame {d} with her placed and standing"
                ),
            ),
            (Some(i), None) => (
                false,
                format!("FAIL the intro (frame {i}) came while covered"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn ready() -> Inputs {
        Inputs {
            busy: false,
            scripts: true,
            standing: true,
            placements: 1,
            pipelines_left: 0,
        }
    }

    /// Ticks until the phase is no longer `Loading`/`Streaming`, counting.
    fn run(c: &mut Cover, i: Inputs, n: u32) {
        for _ in 0..n {
            c.tick(i, DT);
        }
    }

    #[test]
    fn up_from_the_first_frame() {
        let c = Cover::raise();
        assert!(c.covered());
        assert_eq!(c.black(), 1.0);
        assert!(c.holds_movies());
        assert!(!Cover::clear().covered());
    }

    #[test]
    fn loading_takes_ten_quiet_ticks_then_streaming_ten_more() {
        let mut c = Cover::raise();
        // The first tick sees placements 0, then she is placed.
        c.tick(
            Inputs {
                placements: 0,
                ..ready()
            },
            DT,
        );
        // Placed: a move, so Loading counts from 0.
        c.tick(ready(), DT);
        assert_eq!(c.state, State::Covered(Phase::Loading { quiet: 0 }));
        run(&mut c, ready(), QUIET_TICKS - 1);
        assert_eq!(
            c.state,
            State::Covered(Phase::Loading {
                quiet: QUIET_TICKS - 1
            })
        );
        c.tick(ready(), DT);
        assert!(matches!(c.state, State::Covered(Phase::Streaming { .. })));
        // InGame on the 11th tick of WaitForStreaming (`10 < count`).
        run(&mut c, ready(), STREAM_TICKS);
        assert!(matches!(c.state, State::Covered(Phase::Streaming { .. })));
        c.tick(ready(), DT);
        assert_eq!(c.state, State::Covered(Phase::Settling));
        c.tick(ready(), DT);
        assert!(matches!(c.state, State::Covered(Phase::Intro { .. })));
        assert!(!c.holds_movies());
    }

    #[test]
    fn a_busy_tick_starts_the_count_again() {
        let mut c = Cover::raise();
        c.tick(
            Inputs {
                placements: 0,
                ..ready()
            },
            DT,
        );
        run(&mut c, ready(), 6);
        c.tick(
            Inputs {
                busy: true,
                ..ready()
            },
            DT,
        );
        assert_eq!(c.state, State::Covered(Phase::Loading { quiet: 0 }));
    }

    #[test]
    fn a_move_while_streaming_goes_back_to_loading() {
        let mut c = Cover::raise();
        c.tick(
            Inputs {
                placements: 0,
                ..ready()
            },
            DT,
        );
        run(&mut c, ready(), QUIET_TICKS + 1);
        assert!(matches!(c.state, State::Covered(Phase::Streaming { .. })));
        c.tick(
            Inputs {
                placements: 2,
                ..ready()
            },
            DT,
        );
        assert_eq!(c.state, State::Covered(Phase::Loading { quiet: 0 }));
    }

    #[test]
    fn streaming_ends_at_thirty_seconds() {
        let mut c = Cover {
            state: State::Covered(Phase::Streaming {
                ticks: 0,
                secs: 0.0,
            }),
            ..Cover::raise()
        };
        // Ticks of 4 s: the time passes 30 s on the 8th, before 10 ticks.
        for n in 1..=8 {
            c.tick(ready(), 4.0);
            if n < 8 {
                assert!(matches!(c.state, State::Covered(Phase::Streaming { .. })));
            }
        }
        assert!(!matches!(c.state, State::Covered(Phase::Streaming { .. })));
    }

    #[test]
    fn never_clear_before_she_is_placed_and_the_pipelines_are_built() {
        let mut c = Cover::raise();
        let waiting = Inputs {
            placements: 0,
            standing: false,
            pipelines_left: 5,
            ..ready()
        };
        // Up to just short of the cap: never ready.
        let ticks = ((CAP - 1.0) / DT) as u32;
        for _ in 0..ticks {
            c.tick(waiting, DT);
            assert!(c.holds_movies(), "{:?} at {}", c.state, c.age);
        }
        // Placed and standing, the pipelines still building.
        let mut c = Cover::raise();
        c.tick(
            Inputs {
                placements: 0,
                ..ready()
            },
            DT,
        );
        run(
            &mut c,
            Inputs {
                pipelines_left: 3,
                ..ready()
            },
            200,
        );
        assert_eq!(c.state, State::Covered(Phase::Settling));
        assert!(c.holds_movies());
        // Built: ready.
        c.tick(ready(), DT);
        assert!(matches!(c.state, State::Covered(Phase::Intro { .. })));
    }

    #[test]
    fn standing_unmoved_counts_as_placed_after_the_wait() {
        let mut c = Cover::raise();
        let still = Inputs {
            placements: 0,
            ..ready()
        };
        run(&mut c, still, ((PLACE_WAIT - 0.5) / DT) as u32);
        assert!(!c.placed);
        run(&mut c, still, (1.0 / DT) as u32);
        assert!(c.placed);
    }

    #[test]
    fn the_cap_lets_it_go() {
        let mut c = Cover::raise();
        let stuck = Inputs {
            busy: true,
            ..ready()
        };
        run(&mut c, stuck, ((CAP + 1.0) / DT) as u32);
        assert!(!c.holds_movies());
    }

    #[test]
    fn fades_into_the_intro_or_wipes_back_without_one() {
        let mut c = Cover {
            state: State::Covered(Phase::Intro { waited: 0.0 }),
            ..Cover::raise()
        };
        assert!(c.movie_started().is_some());
        assert_eq!(c.black(), 1.0);
        c.frame(FADE / 2.0);
        assert!((c.black() - 0.5).abs() < 1e-3);
        c.frame(FADE);
        assert_eq!(c.state, State::Clear);

        let mut c = Cover {
            state: State::Covered(Phase::Intro { waited: 0.0 }),
            ..Cover::raise()
        };
        run(&mut c, ready(), (INTRO_WAIT / DT) as u32 + 1);
        assert_eq!(c.state, State::Clear);
    }

    #[test]
    fn the_check_fails_on_a_world_frame_before_the_intro() {
        let mut k = Check::default();
        let up = Cover::raise();
        k.frame(&up, false, false, false, false);
        k.frame(&up, true, false, true, true);
        let mut clear = Cover::clear();
        k.frame(&clear, true, true, true, true);
        k.intro();
        let (ok, line) = k.verdict();
        assert!(!ok && line.contains("frame 2"), "{line}");

        let mut k = Check::default();
        k.frame(&up, false, false, false, false);
        k.frame(&up, true, false, true, true);
        clear.state = State::FadingOut { left: FADE };
        k.intro();
        k.frame(&clear, true, false, true, true);
        let (ok, line) = k.verdict();
        assert!(ok, "{line}");

        let mut k = Check::default();
        k.frame(&up, false, false, false, false);
        assert!(!k.verdict().0);
    }
}
