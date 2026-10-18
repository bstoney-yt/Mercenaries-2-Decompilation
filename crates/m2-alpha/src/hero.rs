//! The three mercenaries the player can be: Mattias Nilsson, Jennifer Mui
//! and Chris Jacobs.
//!
//! The game keeps each as a template of the template database (`mattias`,
//! `Jen`, `Chris`; heroes round 02): its `ModelName` is the body
//! (`pmc_hum_mattias`, `pmc_hum_jen`, `pmc_hum_chris`), its `Health` how
//! much it takes (120, 100, 100), its `HumanAnimationSet`'s
//! `CharacterName` (`mattias`, `Jennifer`, `Chris`) picks the clip group of
//! its own (`characternameanimgroup_<name>`) laid over the clips every
//! person shares, and its voice set is `vo_mattias`, `vo_Jen` or
//! `vo_Chris`. The scripts tell the heroes apart by the player
//! character's name.

use std::fmt;

use m2_assets::name_hash;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hero {
    Mattias,
    #[default]
    Jennifer,
    Chris,
}

impl Hero {
    /// The order the front end offers them in.
    pub const ALL: [Hero; 3] = [Hero::Mattias, Hero::Jennifer, Hero::Chris];

    /// The name the scripts and the template database know the hero by.
    /// Full name, as the pause menu's profile line shows it (Mattias's as
    /// in the PC footage; the others' full names, not checked there).
    pub fn full_name(self) -> &'static str {
        match self {
            Hero::Mattias => "Mattias Nilsson",
            Hero::Jennifer => "Jennifer Mui",
            Hero::Chris => "Chris Jacobs",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Hero::Mattias => "Mattias",
            Hero::Jennifer => "Jennifer",
            Hero::Chris => "Chris",
        }
    }

    /// The hero named `name` (any case; `mattias`, `jen`, `chris`).
    pub fn parse(name: &str) -> Option<Self> {
        let n = name.trim().to_ascii_lowercase();
        Hero::ALL.into_iter().find(|h| {
            let full = h.name().to_ascii_lowercase();
            n == full || (n.len() >= 3 && full.starts_with(&n))
        })
    }

    /// The hero's template in the template database.
    pub fn template(self) -> &'static str {
        match self {
            Hero::Mattias => "mattias",
            Hero::Jennifer => "Jen",
            Hero::Chris => "Chris",
        }
    }

    /// The hero's own clip group's block-path word: the group the
    /// `HumanAnimationSet`'s `CharacterName` names.
    pub fn clip_group(self) -> String {
        format!("{GROUP_PREFIX}{}", self.name().to_ascii_lowercase())
    }

    /// The hero's voice set, by name hash.
    pub fn voice_set(self) -> u32 {
        name_hash(match self {
            Hero::Mattias => "vo_mattias",
            Hero::Jennifer => "vo_Jen",
            Hero::Chris => "vo_Chris",
        })
    }
}

impl fmt::Display for Hero {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What the block paths of the heroes' own clip groups start with.
pub const GROUP_PREFIX: &str = "characternameanimgroup_";

/// Whether a clip block at `path` is for `hero`: shared by everyone, or
/// the hero's own group (not another hero's).
pub fn block_for(path: &str, hero: Hero) -> bool {
    let path = path.to_ascii_lowercase();
    match path.find(GROUP_PREFIX) {
        None => true,
        Some(at) => {
            let rest = &path[at + GROUP_PREFIX.len()..];
            let own = hero.name().to_ascii_lowercase();
            // Only the three heroes' groups are told apart; any other
            // character's group is shared as before.
            let named = Hero::ALL
                .into_iter()
                .find(|h| rest.starts_with(&h.name().to_ascii_lowercase()));
            named.is_none_or(|_| rest.starts_with(&own))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse() {
        assert_eq!(Hero::parse("MATTIAS"), Some(Hero::Mattias));
        assert_eq!(Hero::parse("jen"), Some(Hero::Jennifer));
        assert_eq!(Hero::parse("chris"), Some(Hero::Chris));
        assert_eq!(Hero::parse("ch"), None);
        assert_eq!(Hero::parse("fiona"), None);
        assert_eq!(Hero::Jennifer.voice_set(), 0x7f74_6c16);
        assert_eq!(Hero::Mattias.voice_set(), 0x8888_2912);
        assert_eq!(Hero::Chris.voice_set(), 0x38a1_c83c);
        assert_eq!(Hero::Mattias.clip_group(), "characternameanimgroup_mattias");
    }

    #[test]
    fn each_hero_keeps_shared_clips_and_its_own_group() {
        let resident = "data/anim/resident";
        let mat = "x/characternameanimgroup_mattias_P000_Q3";
        let jen = "x/characternameanimgroup_jennifer_P000_Q3";
        let other = "x/characternameanimgroup_soldier";
        for h in Hero::ALL {
            assert!(block_for(resident, h));
            assert!(block_for(other, h));
        }
        assert!(block_for(mat, Hero::Mattias));
        assert!(!block_for(mat, Hero::Jennifer));
        assert!(block_for(jen, Hero::Jennifer));
        assert!(!block_for(jen, Hero::Chris));
    }
}
