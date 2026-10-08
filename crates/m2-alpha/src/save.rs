//! Saved games, in our own format: a small TOML file of where the player
//! is, her health and weapon, where the vehicles are and which are wrecked,
//! and which soldiers are dead.
//!
//! The game's own saves are not read or written. Nothing from the install
//! goes in a save: vehicles and soldiers are named by their asset and
//! entity hashes.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use m2_math::glam::Vec3;
use serde::{Deserialize, Serialize};

/// Bumped when a save from an older build can no longer be read.
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "m2-alpha-save.toml";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveGame {
    pub version: u32,
    pub player: PlayerSave,
    pub vehicles: Vec<VehicleSave>,
    /// Spawn entities of the soldiers killed.
    pub killed: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerSave {
    /// Feet, world space.
    pub position: [f32; 3],
    pub facing: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub hp: f32,
    /// What she has up: "rifle", "rpg" or "grenade launcher".
    pub weapon: String,
    /// What she carries in the launcher's place: "rpg", "grenade
    /// launcher" or nothing (""); the RPG in saves from before it could
    /// be anything else.
    #[serde(default = "rpg")]
    pub second: String,
    /// The grenade launcher's rounds, in its clip and stored, if she
    /// carries one.
    #[serde(default)]
    pub launcher: Option<[u32; 2]>,
    /// Grenades she has; saves from before they ran out had her full.
    #[serde(default = "full_grenades")]
    pub grenades: u32,
    /// The rifle's rounds, in its clip and stored; saves from before it
    /// ran out had it full.
    #[serde(default)]
    pub rifle: Option<[u32; 2]>,
    /// The templates of the guns in her hands' two places (her rifle's,
    /// and what she carries in the launcher's), and the explosive she has
    /// chosen with how many of C4 and mines she has; saves from before
    /// had the carbine or her own, and none of either.
    #[serde(default)]
    pub guns: [String; 2],
    #[serde(default)]
    pub explosives: [u32; 3],
}

fn full_grenades() -> u32 {
    crate::combat::MAX_GRENADES
}

fn rpg() -> String {
    "rpg".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VehicleSave {
    /// Model asset hash.
    pub model: u32,
    /// Model origin, world space.
    pub position: [f32; 3],
    /// Heading, radians clockwise from +z seen from above.
    pub yaw: f32,
    pub wrecked: bool,
}

impl VehicleSave {
    pub fn position(&self) -> Vec3 {
        Vec3::from(self.position)
    }
}

impl SaveGame {
    /// Where saves go (see [`Config::file_beside`](crate::config::Config::file_beside)).
    pub fn default_path() -> PathBuf {
        crate::config::Config::file_beside(FILE_NAME)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("a save always serialises")
    }

    pub fn from_toml(text: &str) -> anyhow::Result<Self> {
        let save: Self = toml::from_str(text)?;
        anyhow::ensure!(
            save.version == VERSION,
            "save version {} (this build reads {VERSION})",
            save.version
        );
        Ok(save)
    }

    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        std::fs::write(path, self.to_toml()).with_context(|| format!("writing {}", path.display()))
    }

    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_toml(&text).with_context(|| format!("reading {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save() -> SaveGame {
        SaveGame {
            version: VERSION,
            player: PlayerSave {
                position: [450.0, -32.5, -1950.0],
                facing: 1.0,
                yaw: 1.1,
                pitch: -0.2,
                hp: 64.0,
                weapon: "grenade launcher".into(),
                second: "grenade launcher".into(),
                launcher: Some([4, 12]),
                rifle: Some([12, 90]),
                grenades: 3,
                guns: ["Shotgun".into(), "Stinger".into()],
                explosives: [0, 2, 1],
            },
            vehicles: vec![VehicleSave {
                model: 0x0307_555e,
                position: [460.0, -32.0, -1940.0],
                yaw: 0.5,
                wrecked: true,
            }],
            killed: vec![7, 9],
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let s = save();
        assert_eq!(SaveGame::from_toml(&s.to_toml()).unwrap(), s);
    }

    #[test]
    fn saves_from_before_the_grenade_launcher_carry_the_rpg() {
        let old = r#"
version = 1
killed = []
vehicles = []

[player]
position = [1.0, 2.0, 3.0]
facing = 0.0
yaw = 0.0
pitch = 0.0
hp = 100.0
weapon = "rpg"
"#;
        let p = SaveGame::from_toml(old).unwrap().player;
        assert_eq!((p.second.as_str(), p.launcher), ("rpg", None));
    }

    #[test]
    fn other_versions_are_refused() {
        let mut s = save();
        s.version = VERSION + 1;
        assert!(SaveGame::from_toml(&s.to_toml()).is_err());
    }
}
