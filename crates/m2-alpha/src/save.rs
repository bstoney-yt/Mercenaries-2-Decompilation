//! Saved games: each slot one file in the 360's save record layout
//! ([`m2_game::save_record`], `m2-alpha-save<n>.sav`), with a small TOML
//! file beside it (`.world.toml`) for what the port puts back that the
//! 360's record does not hold.
//!
//! # Provenance
//!
//! **From the 360 code** ([`m2_game::save_record`]): the `.sav` file's
//! bytes, 0x3458 of them; what goes in it (her cash and fuel, the time
//! played, the last contract and percent done as `Sys.RequestAutosave`
//! last gave them, when it was saved, the character, whether it was the
//! autosave, the options block, the scripts' save text packed with zlib);
//! and that loading hands the save text to the scripts
//! ([`MissionLink::with_save`](crate::mission::MissionLink::with_save)).
//!
//! **Invented**:
//!
//! - The file names and that the slots are files beside the settings
//!   (the 360 keeps them on its storage device, each named by the
//!   profile's name, `auto_XXXXXXXX`).
//! - The owner's id ([`m2_game::save_record::OWNER`]).
//! - The `.world.toml` beside it: where she is, her facing and camera,
//!   her health, her weapons and their rounds, grenades and explosives,
//!   the island's vehicles with their places and wrecks, the soldiers
//!   killed, and her support stock. The 360 saves none of these: its load
//!   (`0x8230C880`, `0x8230C740`) resets the players (out of their seats,
//!   out of survival mode) and leaves the rest to the scripts'
//!   `LoadSingleton`. The port still puts them back, from that file, until
//!   a check in the game shows where the scripts alone put her.
//! - The options block is written at its defaults: the port keeps the
//!   player's options in its own settings file.
//!
//! Saves of the port's own TOML format (`m2-alpha-save<n>.toml`, from
//! before) are still read, and are replaced by the record when the slot
//! is saved over. Nothing from the install goes in a save: vehicles and
//! soldiers are named by their asset and entity hashes.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use m2_game::save_record::{RecordInfo, SaveRecord};
use m2_math::glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::hero::Hero;

/// The old TOML saves' version, and the `.world.toml`'s: bumped when one
/// from an older build can no longer be read.
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "m2-alpha-save.sav";
/// The record's extension.
pub const EXT: &str = "sav";
/// The port's old TOML saves' extension.
pub const LEGACY_EXT: &str = "toml";
/// The invented world file's extension, beside the record.
pub const WORLD_EXT: &str = "world.toml";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveGame {
    pub version: u32,
    /// Who she is (the record's character); old saves from before had none
    /// (the settings' hero stands in).
    #[serde(default)]
    pub hero: Option<Hero>,
    /// Where she is and what she carries (**invented**: not in the 360's
    /// record); none for a record without its world file.
    #[serde(default)]
    pub player: Option<PlayerSave>,
    #[serde(default)]
    pub vehicles: Vec<VehicleSave>,
    /// Spawn entities of the soldiers killed.
    #[serde(default)]
    pub killed: Vec<u32>,
    /// Her cash, fuel (the record's) and support stock (the world file's)
    /// as the scripts held them; old saves from before money have none.
    #[serde(default)]
    pub pmc: Option<PmcSave>,
    /// The scripts' own save data, as Lua text
    /// ([`m2_game::savedata`]): loading hands it back to the scripts as
    /// they start ([`MissionLink::with_save`](crate::mission::MissionLink::with_save)),
    /// so the contracts done and on offer come back with it.
    #[serde(default)]
    pub scripts: Option<String>,
    /// The record's own fields (play time, contract, percent, when, the
    /// autosave flag); all nought for an old TOML save.
    #[serde(skip)]
    pub info: RecordInfo,
}

/// What her PMC has: cash, fuel and the support items she holds, by
/// their key in the scripts' catalogue (`"clusterbomb"`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PmcSave {
    pub cash: f64,
    pub fuel: f64,
    pub stock: Vec<(String, i64)>,
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

/// The invented world file beside a record (see the module's provenance).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct WorldFile {
    version: u32,
    #[serde(default)]
    player: Option<PlayerSave>,
    #[serde(default)]
    vehicles: Vec<VehicleSave>,
    #[serde(default)]
    killed: Vec<u32>,
    #[serde(default)]
    stock: Vec<(String, i64)>,
}

/// What a save's invented world file held, taken out of it
/// ([`strip_world`]).
#[derive(Clone, Debug, PartialEq)]
pub struct WorldRef {
    pub player: Option<PlayerSave>,
    pub vehicles: usize,
    pub killed: usize,
}

/// Leaves `save` as the 360's record alone would load it: no place,
/// weapons, vehicles, dead or support stock (none of which the record
/// holds). What it had is returned.
pub fn strip_world(save: &mut SaveGame) -> WorldRef {
    if let Some(p) = &mut save.pmc {
        p.stock.clear();
    }
    WorldRef {
        player: save.player.take(),
        vehicles: std::mem::take(&mut save.vehicles).len(),
        killed: std::mem::take(&mut save.killed).len(),
    }
}

/// The record's character for `hero`: the save list movie's 1 Mattias,
/// 2 Chris, 3 Jennifer.
pub fn character(hero: Hero) -> u8 {
    match hero {
        Hero::Mattias => 1,
        Hero::Chris => 2,
        Hero::Jennifer => 3,
    }
}

/// The hero of the record's character `c`, if it is one.
pub fn hero_of(c: u8) -> Option<Hero> {
    match c {
        1 => Some(Hero::Mattias),
        2 => Some(Hero::Chris),
        3 => Some(Hero::Jennifer),
        _ => None,
    }
}

/// Seconds since 1970 now, as `time(0)` stamps the record.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl SaveGame {
    /// Where saves go (see [`Config::file_beside`](crate::config::Config::file_beside)).
    pub fn default_path() -> PathBuf {
        crate::config::Config::file_beside(FILE_NAME)
    }

    /// Where the scripts' saves in slot `slot` go: the autosaves
    /// (`"autosave"`) beside the player's own, `m2-alpha-autosave.sav`.
    pub fn slot_path(slot: &str) -> PathBuf {
        let slot: String = slot.chars().filter(char::is_ascii_alphanumeric).collect();
        crate::config::Config::file_beside(&format!("m2-alpha-{}.{EXT}", slot.to_ascii_lowercase()))
    }

    /// The file a save at `path` is in: the record, else an old TOML save
    /// of the same name; `None` if neither is there.
    pub fn found(path: &Path) -> Option<PathBuf> {
        let record = record_path(path);
        if record.is_file() {
            return Some(record);
        }
        let legacy = record.with_extension(LEGACY_EXT);
        legacy.is_file().then_some(legacy)
    }

    /// When the save at `path` was written ([`found`](Self::found)).
    pub fn modified(path: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::found(path)?).ok()?.modified().ok()
    }

    /// The newest of the player's save and the autosave: what loading
    /// loads.
    pub fn newest_path() -> PathBuf {
        let own = Self::default_path();
        let auto = Self::slot_path(m2_game::api::AUTOSAVE);
        match (Self::modified(&own), Self::modified(&auto)) {
            (Some(a), Some(b)) if b > a => auto,
            (None, Some(_)) => auto,
            _ => own,
        }
    }

    /// The old TOML format's text (the tests write old saves with it).
    #[cfg(test)]
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("a save always serialises")
    }

    /// An old TOML save.
    pub fn from_toml(text: &str) -> anyhow::Result<Self> {
        let save: Self = toml::from_str(text)?;
        anyhow::ensure!(
            save.version == VERSION,
            "save version {} (this build reads {VERSION})",
            save.version
        );
        Ok(save)
    }

    /// The record's fields for this save, stamped `saved_at`.
    pub fn record_info(&self, saved_at: u64) -> RecordInfo {
        let pmc = self.pmc.clone().unwrap_or_default();
        RecordInfo {
            cash: pmc.cash.max(0.0).round().min(f64::from(u32::MAX)) as u32,
            fuel: pmc.fuel.round() as i32,
            saved_at,
            character: self.hero.map_or(0, character),
            ..self.info.clone()
        }
    }

    /// The record's bytes, stamped `saved_at`.
    pub fn to_record(&self, saved_at: u64) -> anyhow::Result<Vec<u8>> {
        let text = self.scripts.as_deref().unwrap_or("");
        let mut record = Box::<SaveRecord>::default();
        record.set_info(&self.record_info(saved_at));
        if !text.is_empty() {
            record.pack(text)?;
        }
        Ok(record.to_bytes())
    }

    /// A save from a record's bytes and, if there is one, its world
    /// file's text.
    pub fn from_record(bytes: &[u8], world: Option<&str>) -> anyhow::Result<Self> {
        let record = SaveRecord::from_bytes(bytes)?;
        let info = record.info();
        let world: Option<WorldFile> = world
            .map(|t| toml::from_str(t).context("its world file"))
            .transpose()?;
        if let Some(w) = &world {
            anyhow::ensure!(
                w.version == VERSION,
                "world file version {} (this build reads {VERSION})",
                w.version
            );
        }
        let world = world.unwrap_or(WorldFile {
            version: VERSION,
            player: None,
            vehicles: Vec::new(),
            killed: Vec::new(),
            stock: Vec::new(),
        });
        Ok(Self {
            version: VERSION,
            hero: hero_of(info.character),
            player: world.player,
            vehicles: world.vehicles,
            killed: world.killed,
            pmc: Some(PmcSave {
                cash: f64::from(info.cash),
                fuel: f64::from(info.fuel),
                stock: world.stock,
            }),
            scripts: record.script_text()?,
            info,
        })
    }

    /// The world file's text.
    fn world_toml(&self) -> String {
        toml::to_string_pretty(&WorldFile {
            version: VERSION,
            player: self.player.clone(),
            vehicles: self.vehicles.clone(),
            killed: self.killed.clone(),
            stock: self
                .pmc
                .as_ref()
                .map(|p| p.stock.clone())
                .unwrap_or_default(),
        })
        .expect("a world file always serialises")
    }

    /// Writes the record (and its world file) for `path`, stamped now; an
    /// old TOML save in that slot goes, the record taking its place.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let record = record_path(path);
        let bytes = self.to_record(now_secs())?;
        std::fs::write(&record, bytes).with_context(|| format!("writing {}", record.display()))?;
        let world = record.with_extension(WORLD_EXT);
        std::fs::write(&world, self.world_toml())
            .with_context(|| format!("writing {}", world.display()))?;
        let legacy = record.with_extension(LEGACY_EXT);
        if legacy.is_file() {
            let _ = std::fs::remove_file(legacy);
        }
        Ok(())
    }

    /// Reads the save at `path`: a record (with its world file if there is
    /// one), or an old TOML save.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let file = Self::found(path).unwrap_or_else(|| path.to_path_buf());
        if is_legacy(&file) {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            return Self::from_toml(&text).with_context(|| format!("reading {}", file.display()));
        }
        let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
        let world = std::fs::read_to_string(file.with_extension(WORLD_EXT)).ok();
        Self::from_record(&bytes, world.as_deref())
            .with_context(|| format!("reading {}", file.display()))
    }
}

/// Whether `path` names an old TOML save (not a record's world file).
fn is_legacy(path: &Path) -> bool {
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    name.ends_with(".toml") && !name.ends_with(&format!(".{WORLD_EXT}"))
}

/// The record's path for a save named `path` (whatever extension it was
/// given).
fn record_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let stem = name
        .strip_suffix(&format!(".{WORLD_EXT}"))
        .or_else(|| name.strip_suffix(&format!(".{LEGACY_EXT}")))
        .or_else(|| name.strip_suffix(&format!(".{EXT}")))
        .unwrap_or(&name);
    path.with_file_name(format!("{stem}.{EXT}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save() -> SaveGame {
        SaveGame {
            version: VERSION,
            hero: Some(Hero::Jennifer),
            player: Some(PlayerSave {
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
            }),
            vehicles: vec![VehicleSave {
                model: 0x0307_555e,
                position: [460.0, -32.0, -1940.0],
                yaw: 0.5,
                wrecked: true,
            }],
            killed: vec![7, 9],
            pmc: Some(PmcSave {
                cash: 125_000.0,
                fuel: 240.0,
                stock: vec![("clusterbomb".into(), 2), ("Support".into(), 1)],
            }),
            scripts: Some("{[\"nAvailableCostumes\"]=1,}".into()),
            info: RecordInfo {
                played: 3725,
                contract: "VzaCon001".into(),
                percent: 4,
                autosave: true,
                ..RecordInfo::default()
            },
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("m2-alpha-save-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_trips_through_the_record_and_its_world_file() {
        let s = save();
        let bytes = s.to_record(1_795_000_000).unwrap();
        assert_eq!(bytes.len(), 0x3458);
        let back = SaveGame::from_record(&bytes, Some(&s.world_toml())).unwrap();
        let mut want = s.clone();
        want.info.cash = 125_000;
        want.info.fuel = 240;
        want.info.saved_at = 1_795_000_000;
        want.info.character = 3;
        assert_eq!(back, want);
    }

    /// Without its world file a record still loads: the 360's own fields.
    #[test]
    fn a_record_alone_has_the_360s_fields_only() {
        let bytes = save().to_record(5).unwrap();
        let s = SaveGame::from_record(&bytes, None).unwrap();
        assert_eq!(s.hero, Some(Hero::Jennifer));
        assert_eq!(s.player, None);
        assert!(s.vehicles.is_empty() && s.killed.is_empty());
        let pmc = s.pmc.unwrap();
        assert_eq!((pmc.cash, pmc.fuel, pmc.stock.len()), (125_000.0, 240.0, 0));
        assert_eq!(s.scripts.as_deref(), Some("{[\"nAvailableCostumes\"]=1,}"));
        assert_eq!(
            (s.info.played, s.info.contract.as_str(), s.info.percent),
            (3725, "VzaCon001", 4)
        );
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
        let s = SaveGame::from_toml(old).unwrap();
        let p = s.player.unwrap();
        assert_eq!((p.second.as_str(), p.launcher), ("rpg", None));
        // From before money and heroes: none kept.
        assert_eq!((s.pmc, s.scripts, s.hero), (None, None, None));
    }

    #[test]
    fn the_scripts_autosaves_go_beside_the_players_own() {
        let auto = SaveGame::slot_path(m2_game::api::AUTOSAVE);
        assert_eq!(auto.file_name().unwrap(), "m2-alpha-autosave.sav");
        assert_eq!(auto.parent(), SaveGame::default_path().parent());
        assert_eq!(
            SaveGame::slot_path("../retry").file_name().unwrap(),
            "m2-alpha-retry.sav"
        );
    }

    #[test]
    fn old_toml_saves_are_read_and_replaced_by_the_record() {
        let d = dir("migrate");
        let slot = d.join("m2-alpha-save3.sav");
        let mut old = save();
        old.info = RecordInfo::default();
        std::fs::write(d.join("m2-alpha-save3.toml"), old.to_toml()).unwrap();
        assert_eq!(SaveGame::found(&slot), Some(d.join("m2-alpha-save3.toml")));
        let read = SaveGame::read(&slot).unwrap();
        assert_eq!(read, old);
        read.write(&slot).unwrap();
        assert!(!d.join("m2-alpha-save3.toml").exists());
        assert_eq!(SaveGame::found(&slot), Some(slot.clone()));
        let again = SaveGame::read(&slot).unwrap();
        assert_eq!(again.player, old.player);
        assert_eq!(again.scripts, old.scripts);
        assert_eq!(again.pmc, old.pmc);
        assert!(again.info.saved_at > 0);
        // A path given as the old file's still finds the record.
        assert_eq!(
            SaveGame::read(&d.join("m2-alpha-save3.toml")).unwrap(),
            again
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn other_versions_are_refused() {
        let mut s = save();
        s.version = VERSION + 1;
        assert!(SaveGame::from_toml(&s.to_toml()).is_err());
        let bytes = save().to_record(1).unwrap();
        let world = save().world_toml().replace("version = 1", "version = 9");
        assert!(SaveGame::from_record(&bytes, Some(&world)).is_err());
    }

    #[test]
    fn stripping_the_world_leaves_the_records_fields() {
        let mut s = save();
        let w = strip_world(&mut s);
        assert_eq!(w.player, save().player);
        assert_eq!((w.vehicles, w.killed), (1, 2));
        assert_eq!(
            (s.player.as_ref(), s.vehicles.len(), s.killed.len()),
            (None, 0, 0)
        );
        let pmc = s.pmc.clone().unwrap();
        assert_eq!((pmc.cash, pmc.fuel, pmc.stock.len()), (125_000.0, 240.0, 0));
        // What a record alone loads.
        let bytes = save().to_record(5).unwrap();
        let alone = SaveGame::from_record(&bytes, None).unwrap();
        assert_eq!((alone.player, alone.scripts), (s.player, s.scripts));
    }

    #[test]
    fn characters_are_the_save_lists() {
        for h in Hero::ALL {
            assert_eq!(hero_of(character(h)), Some(h));
        }
        assert_eq!(character(Hero::Chris), 2);
        assert_eq!(hero_of(0), None);
    }
}
