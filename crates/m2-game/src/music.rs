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
    /// The region's faction takes over (since `ActivateFactionRegionMusic`).
    following: bool,
    /// The faction (name hash) of the music region the player is in.
    region: Option<u32>,
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
