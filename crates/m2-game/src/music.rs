//! The score: music states, the cues bound to them, and which plays.
//!
//! `MrxMusic` sets the music up per faction (`an`, `pmc`, ...) and per
//! free-play area (`freeplay_city`, ...): `AddFactionMusic` declares one,
//! `SetFactionMusic` picks the one later bindings and playback use, and
//! `BindMusicCue(cue, state)` ties a cue to a state of it. Scripts then move
//! between states with `TransitionMusic`; a contract locks its faction
//! (`LockFactionMusic`), and special pieces are bound to the `misc` states
//! and played through them. A state with no cue (`none`, `silence`) is
//! quiet.
//!
//! In free play the score follows the player: the music regions
//! ([`m2_engine::sound_regions`]) name `freeplay_jungle` or
//! `freeplay_water` where they lie, and the root faction (`freeplay_city`)
//! plays outside them, once `ActivateFactionRegionMusic` has been called
//! and while the faction is not locked. Changing faction keeps the state
//! and plays the new faction's cue for it (inferred; the game's
//! transition timing between factions is not decoded).
//!
//! With dynamic music on, the score also moves by itself: each faction's
//! states and transitions are a machine ([`crate::music_machine`], from
//! the 360 code) that leaves a state when its length has passed and
//! weighs it against the action level at its interval. What the scripts
//! ask for (`TransitionMusic`) still plays at once here, where the 360
//! takes it up on its machine's next run and after the state's delay.
//!
//! Not done: the action level's counts (the game counts nobody yet, so
//! combat does not raise the score), a piece running out (the game does
//! not say when), and interrupting states giving back their piece's
//! place. The first contract turns dynamic music off as it starts.
//!
//! The radios' playlists are kept here too ([`Music::playlist`]); the game
//! plays one near its radio (`m2_engine::music_source`).

use std::collections::HashMap;

use crate::api::Command;
use crate::music_machine::{Event, Faction, Hostility};

/// The music's state as the scripts set it.
#[derive(Debug, Default)]
pub struct Music {
    /// Bindings by faction, then state: the cues it may play.
    bindings: HashMap<String, HashMap<String, Vec<String>>>,
    /// The faction bindings and playback use.
    faction: String,
    /// The free-play faction used when no region names one.
    root: String,
    pub locked: bool,
    /// The region's faction takes over (since `ActivateFactionRegionMusic`).
    following: bool,
    /// The faction (name hash) of the music region the player is in.
    region: Option<u32>,
    /// Dynamic music is off (`Sound.SetDynamicMusic(false)`). It starts
    /// out on, as the 360's factions do (`0x8235B2D0`).
    pub fixed: bool,
    state: String,
    /// The cue playing, if any.
    playing: Option<String>,
    /// Picks among a state's cues.
    turn: usize,
    /// The radios' playlists (`Sound.AddMusicSourcePlaylist`, filled by
    /// `AddCueToMusicSourcePlaylist`), by name hash: their cues in order.
    playlists: HashMap<u32, Vec<String>>,
    /// Each faction's states and transitions, and where it is
    /// ([`crate::music_machine`]).
    machines: HashMap<String, Faction>,
    /// The action around the player, which the machines weigh.
    pub hostility: Hostility,
    /// `Sound.LockActionLevelMusic`: the action level stands as it is.
    pub action_locked: bool,
}

impl Music {
    pub fn set_faction(&mut self, faction: &str) {
        self.faction = faction.to_ascii_lowercase();
    }

    pub fn set_root(&mut self, faction: &str) {
        self.root = faction.to_ascii_lowercase();
    }

    /// The region's faction takes over from now on, while the faction
    /// isn't locked (`ActivateFactionRegionMusic`, as free play starts).
    pub fn activate_region(&mut self) -> Option<Command> {
        self.following = true;
        self.follow()
    }

    /// The player is in the music region naming faction `faction` (a name
    /// hash; `None`: in none, where the root faction plays). When the
    /// faction changes the score goes on in the same state with the new
    /// faction's cues: returns the command when what plays changes.
    pub fn enter_region(&mut self, faction: Option<u32>) -> Option<Command> {
        self.region = faction;
        self.follow()
    }

    /// The faction the region asks for: the bound faction whose name
    /// hashes to the region's, else the root.
    fn region_faction(&self) -> Option<String> {
        self.region
            .and_then(|h| {
                self.bindings
                    .keys()
                    .find(|f| m2_assets::name_hash(f) == h)
                    .cloned()
            })
            .or_else(|| (!self.root.is_empty()).then(|| self.root.clone()))
    }

    fn follow(&mut self) -> Option<Command> {
        if !self.following || self.locked {
            return None;
        }
        let faction = self.region_faction()?;
        if faction == self.faction {
            return None;
        }
        self.faction = faction;
        if self.state.is_empty() {
            return None;
        }
        let state = self.state.clone();
        self.transition(&state)
    }

    /// Unlocks or locks the faction; unlocked, the region's faction takes
    /// over again.
    pub fn lock(&mut self, locked: bool) -> Option<Command> {
        self.locked = locked;
        self.follow()
    }

    /// The faction playing and binding now.
    pub fn faction(&self) -> &str {
        &self.faction
    }

    pub fn bind(&mut self, cue: &str, state: &str) {
        self.bindings
            .entry(self.faction.clone())
            .or_default()
            .entry(state.to_ascii_lowercase())
            .or_default()
            .push(cue.to_owned());
    }

    pub fn clear(&mut self, state: &str) {
        if let Some(f) = self.bindings.get_mut(&self.faction) {
            f.remove(&state.to_ascii_lowercase());
        }
    }

    /// A radio playlist named `name` (`mu_src_radio`), empty.
    pub fn add_playlist(&mut self, name: &str) {
        self.playlists
            .insert(m2_assets::name_hash(name), Vec::new());
    }

    /// Cue `cue` goes on the end of playlist `name`.
    pub fn add_to_playlist(&mut self, name: &str, cue: &str) {
        self.playlists
            .entry(m2_assets::name_hash(name))
            .or_default()
            .push(cue.to_owned());
    }

    pub fn clear_playlist(&mut self, name: &str) {
        if let Some(p) = self.playlists.get_mut(&m2_assets::name_hash(name)) {
            p.clear();
        }
    }

    /// The cues of the playlist with name hash `hash` (a radio's).
    pub fn playlist(&self, hash: u32) -> &[String] {
        self.playlists.get(&hash).map_or(&[], Vec::as_slice)
    }

    /// The machine of the faction in use.
    pub fn machine(&mut self) -> &mut Faction {
        self.machines.entry(self.faction.clone()).or_default()
    }

    /// `Sound.SetActionLevelsMusic(a, b, c, d)`: the four values of the
    /// action around the player, the last argument the first value
    /// (`0x82341A88`).
    pub fn set_action_levels(&mut self, a: f32, b: f32, c: f32, d: f32) {
        self.hostility.values = [d, c, b, a];
    }

    /// Runs the faction's machine for `dt` seconds and goes to the state
    /// it moves to by itself, if any: at its interval by the action
    /// level, when its length has passed, or when its piece is over.
    /// `counts` are the hostile characters aware of the player in the
    /// engine's four groups; `decay` is the hostility decay rate.
    ///
    /// The action follows the counts only with dynamic music on and the
    /// level unlocked (`0x82362898`). A piece counts as playing while
    /// one is set here: the game does not say yet when one has run out.
    pub fn update(
        &mut self,
        dt: f32,
        decay: f32,
        counts: [u32; 4],
        clocks: bool,
        force: bool,
        draw: impl FnMut() -> f32,
    ) -> Option<Command> {
        let (dynamic, locked) = (!self.fixed, self.action_locked);
        if dynamic && !locked {
            self.hostility.update(dt, decay, counts);
        }
        let (hostility, playing) = (self.hostility, self.playing.is_some());
        let machine = self.machine();
        machine.dynamic = dynamic;
        machine.clocks = clocks;
        machine.force |= force;
        let events = machine.update(dt, &hostility, locked, false, playing, draw);
        let level = machine.level;
        let mut command = None;
        for e in events {
            if let Event::Entered(state) = e
                && state != self.state
            {
                eprintln!(
                    "music: the score moves from {} to {state} by itself (action level {level}, \
                     hostility {:?})",
                    self.state, hostility.values
                );
                command = self.go(&state).or(command);
            }
        }
        command
    }

    /// Goes to `state`: returns the command for the game when what plays
    /// changes.
    pub fn transition(&mut self, state: &str) -> Option<Command> {
        // The machine takes it up at its next run (`0x82341570`); what
        // plays changes at once here.
        self.machine().ask(state);
        self.go(state)
    }

    fn go(&mut self, state: &str) -> Option<Command> {
        self.state = state.to_ascii_lowercase();
        let cues: Vec<String> = self
            .bindings
            .get(&self.faction)
            .and_then(|f| f.get(&self.state))
            .cloned()
            .unwrap_or_default();
        // Staying in a state keeps its piece going.
        if self.playing.as_ref().is_some_and(|p| cues.contains(p)) {
            return None;
        }
        let cue = (!cues.is_empty()).then(|| {
            self.turn += 1;
            cues[self.turn % cues.len()].clone()
        });
        if cue == self.playing {
            return None;
        }
        self.playing.clone_from(&cue);
        Some(Command::Music(cue))
    }

    /// Cue `cue` now plays (or nothing does) because other scripts said
    /// so: the pause menu's own (`MrxSound.EnterPauseState` and
    /// `ExitPauseState`, run by the HUD's scripts). A later transition
    /// here starts its state's piece again, not keeping a piece that no
    /// longer plays.
    pub fn heard(&mut self, cue: Option<String>) {
        self.playing = cue;
    }

    /// The state the scripts last asked for.
    pub fn state(&self) -> &str {
        &self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_play_their_factions_cues() {
        let mut m = Music::default();
        for f in ["an", "pmc"] {
            m.set_faction(f);
            m.bind(&format!("mu_fac_{f}_explore_01"), "explore");
        }
        m.set_faction("pmc");
        m.locked = true;
        assert_eq!(
            m.transition("explore"),
            Some(Command::Music(Some("mu_fac_pmc_explore_01".into())))
        );
        assert_eq!(m.transition("explore"), None);
        m.clear("misc1");
        m.bind("mu_nomission_water_threat_01", "misc1");
        assert_eq!(
            m.transition("misc1"),
            Some(Command::Music(Some("mu_nomission_water_threat_01".into())))
        );
        assert_eq!(m.transition("none"), Some(Command::Music(None)));
        assert_eq!(m.transition("silence"), None);
        assert_eq!(m.state(), "silence");
    }

    #[test]
    fn the_score_moves_by_itself_when_a_state_s_length_has_passed() {
        use crate::music_machine::State;
        let mut m = Music::default();
        m.set_faction("freeplay_city");
        let state = |name: &str, length: f32| State {
            name: name.into(),
            length,
            level: 0,
            delay: 0.0,
            interval: 10.0,
            least: 0.0,
            flags: 0,
            thresholds: (2.0, 0.0),
            transitions: Vec::new(),
        };
        m.machine().add_state(state("none", 15.0));
        m.machine().add_state(state("explore", 30.0));
        // Timeout transitions (kind 1), as the scripts declare them.
        assert!(m.machine().add_transition("none", "explore", 1.0, 0, 1));
        assert!(m.machine().add_transition("explore", "none", 1.0, 0, 1));
        m.bind("mu_explore_01", "explore");
        assert_eq!(m.transition("none"), None);
        let mut moved = Vec::new();
        for k in 1..=500 {
            if let Some(c) = m.update(0.1, 1.0, [0; 4], true, false, || 0.5) {
                moved.push((k as f32 * 0.1, c, m.state().to_owned()));
            }
        }
        // 15 s of "none", then "explore" and its piece; 30 s later
        // "none" again, and silence.
        assert_eq!(moved.len(), 2, "{moved:?}");
        assert!((moved[0].0 - 15.2).abs() < 0.21, "{moved:?}");
        assert_eq!(moved[0].1, Command::Music(Some("mu_explore_01".into())));
        assert_eq!(moved[0].2, "explore");
        assert!((moved[1].0 - 45.3).abs() < 0.31, "{moved:?}");
        assert_eq!(moved[1].1, Command::Music(None));
        // With dynamic music off, or the clocks stopped, it stays put.
        m.fixed = true;
        m.transition("none");
        assert!((0..500).all(|_| m.update(0.1, 1.0, [0; 4], true, false, || 0.5).is_none()));
        m.fixed = false;
        assert!((0..500).all(|_| m.update(0.1, 1.0, [0; 4], false, false, || 0.5).is_none()));
    }

    #[test]
    fn after_the_pause_menus_silence_the_state_plays_again() {
        let mut m = Music::default();
        m.set_faction("pmc");
        m.bind("mu_fac_pmc_explore_01", "explore");
        assert!(m.transition("explore").is_some());
        // The HUD's scripts: `pause` (`mu_shell_01`), then `silence`.
        m.heard(Some("mu_shell_01".into()));
        m.heard(None);
        assert_eq!(
            m.transition("explore"),
            Some(Command::Music(Some("mu_fac_pmc_explore_01".into())))
        );
    }

    #[test]
    fn free_play_follows_the_region_until_the_faction_is_locked() {
        let mut m = Music::default();
        for f in ["freeplay_city", "freeplay_jungle", "freeplay_water", "pmc"] {
            m.set_faction(f);
            m.bind(&format!("mu_{f}_explore"), "explore");
        }
        m.set_root("freeplay_city");
        let cue = |c: &str| Some(Command::Music(Some(c.to_owned())));
        // Regions count only once free play activates them.
        let jungle = Some(m2_assets::name_hash("freeplay_jungle"));
        assert_eq!(m.enter_region(jungle), None);
        assert_eq!(m.faction(), "pmc");
        assert_eq!(m.activate_region(), None, "no state yet");
        assert_eq!(m.faction(), "freeplay_jungle");
        assert_eq!(m.transition("explore"), cue("mu_freeplay_jungle_explore"));
        // Into the water region, then out of every region: the root.
        let water = Some(m2_assets::name_hash("Freeplay_Water"));
        assert_eq!(m.enter_region(water), cue("mu_freeplay_water_explore"));
        assert_eq!(m.enter_region(water), None);
        assert_eq!(m.enter_region(None), cue("mu_freeplay_city_explore"));
        // A contract locks its faction: regions wait until it is unlocked.
        m.set_faction("pmc");
        assert_eq!(m.lock(true), None);
        assert_eq!(m.transition("explore"), cue("mu_pmc_explore"));
        assert_eq!(m.enter_region(water), None);
        assert_eq!(m.lock(false), cue("mu_freeplay_water_explore"));
    }
}
