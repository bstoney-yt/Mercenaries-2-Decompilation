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
//! Not done: the engine's own action level (combat raising the score to
//! `action` while dynamic music is on), the state timings and the
//! transitions' sync kinds. The first contract turns dynamic music off as
//! it starts, so it needs none of it.
//!
//! The radios' playlists are kept here too ([`Music::playlist`]); the game
//! plays one near its radio (`m2_engine::music_source`).

use std::collections::HashMap;

use crate::api::Command;

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
    pub dynamic: bool,
    state: String,
    /// The cue playing, if any.
    playing: Option<String>,
    /// Picks among a state's cues.
    turn: usize,
    /// The radios' playlists (`Sound.AddMusicSourcePlaylist`, filled by
    /// `AddCueToMusicSourcePlaylist`), by name hash: their cues in order.
    playlists: HashMap<u32, Vec<String>>,
}

impl Music {
    pub fn set_faction(&mut self, faction: &str) {
        self.faction = faction.to_ascii_lowercase();
    }

    pub fn set_root(&mut self, faction: &str) {
        self.root = faction.to_ascii_lowercase();
    }

    /// The region's faction takes over (free play): the root one, as
    /// regions are not read.
    pub fn activate_region(&mut self) {
        if !self.locked && !self.root.is_empty() {
            self.faction = self.root.clone();
        }
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

    /// Goes to `state`: returns the command for the game when what plays
    /// changes.
    pub fn transition(&mut self, state: &str) -> Option<Command> {
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
}
