//! Saved games: each slot one file in the 360's save record layout
//! ([`m2_game::save_record`], `m2-alpha-save<n>.sav`), and nothing else.
//!
//! # Provenance
//!
//! **From the 360 code** ([`m2_game::save_record`]): the `.sav` file's
//! bytes, 0x3458 of them; what goes in it (her cash and fuel, the time
//! played, the last contract and percent done as the scripts last gave
//! them, when it was saved, the character, whether it was the autosave,
//! the options block, the scripts' save text packed with zlib); and what
//! loading does: the record's fields come back and the save text is handed
//! to the scripts as they start
//! ([`MissionLink::with_save`](crate::mission::MissionLink::with_save)),
//! after the world's reset ([`Game::reset_for_load`](crate::game::Game::reset_for_load)).
//! Where she is, her health, weapons and rounds, the vehicles, wrecks and
//! the dead are not saved: the 360's record holds none of them, and its
//! load (`0x8239BE98`, `0x8230C880`, `0x8230C740`, then `LoadSingleton`)
//! leaves them to the scripts. Checked in the game (#783): loaded the
//! 360's way she is back at the scripted start, at full health and ammo.
//!
//! **Invented**:
//!
//! - The file names and that the slots are files beside the settings
//!   (the 360 keeps them on its storage device, each named by the
//!   profile's name, `auto_XXXXXXXX`).
//! - The owner's id ([`m2_game::save_record::OWNER`]).
//! - The options block is written at its defaults: the port keeps the
//!   player's options in its own settings file.
//!
//! Saves of the port's older formats are still read for what the 360's
//! record carries (hero, cash, fuel, the scripts' save text): its own
//! TOML saves (`m2-alpha-save<n>.toml`), whose world part is ignored, and
//! the `.world.toml` that #781-#783 wrote beside each record, which is
//! never read. Saving over a slot removes both.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use m2_game::save_record::{RecordInfo, SaveRecord};
use serde::{Deserialize, Serialize};

use crate::hero::Hero;

/// The old TOML saves' version: one from another build is refused.
pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "m2-alpha-save.sav";
/// The record's extension.
pub const EXT: &str = "sav";
/// The port's old TOML saves' extension.
pub const LEGACY_EXT: &str = "toml";
/// The world file #781-#783 wrote beside each record: never read now,
/// removed when the slot is saved over.
pub const STALE_WORLD_EXT: &str = "world.toml";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveGame {
    pub version: u32,
    /// Who she is (the record's character); old saves from before had none
    /// (the settings' hero stands in).
    #[serde(default)]
    pub hero: Option<Hero>,
    /// Her cash and fuel (the record's); the support stock only in old
    /// TOML saves from before the scripts' data was kept (for those the
    /// scripts have nothing else to restore it from). Old saves from before
    /// money have none.
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
    #[serde(default)]
    pub stock: Vec<(String, i64)>,
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
    /// A save of `hero` with nothing else yet (the scripts' part and the
    /// record's stamp are filled in by whoever writes it).
    pub fn of(hero: Hero) -> Self {
        Self {
            version: VERSION,
            hero: Some(hero),
            pmc: None,
            scripts: None,
            info: RecordInfo::default(),
        }
    }

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

    /// An old TOML save: its hero, cash, fuel, stock and the scripts' data
    /// (the world part it also had is left out).
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

    /// A save from a record's bytes.
    pub fn from_record(bytes: &[u8]) -> anyhow::Result<Self> {
        let record = SaveRecord::from_bytes(bytes)?;
        let info = record.info();
        Ok(Self {
            version: VERSION,
            hero: hero_of(info.character),
            pmc: Some(PmcSave {
                cash: f64::from(info.cash),
                fuel: f64::from(info.fuel),
                stock: Vec::new(),
            }),
            scripts: record.script_text()?,
            info,
        })
    }

    /// Writes the record for `path`, stamped now; an old TOML save or a
    /// stale world file in that slot goes, the record taking its place.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let record = record_path(path);
        let bytes = self.to_record(now_secs())?;
        std::fs::write(&record, bytes).with_context(|| format!("writing {}", record.display()))?;
        for ext in [LEGACY_EXT, STALE_WORLD_EXT] {
            let old = record.with_extension(ext);
            if old.is_file() {
                let _ = std::fs::remove_file(old);
            }
        }
        Ok(())
    }

    /// Reads the save at `path`: a record, or an old TOML save.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let file = Self::found(path).unwrap_or_else(|| path.to_path_buf());
        if is_legacy(&file) {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            return Self::from_toml(&text).with_context(|| format!("reading {}", file.display()));
        }
        let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
        Self::from_record(&bytes).with_context(|| format!("reading {}", file.display()))
    }
}

/// Whether `path` names an old TOML save (not a stale world file).
fn is_legacy(path: &Path) -> bool {
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    name.ends_with(".toml") && !name.ends_with(&format!(".{STALE_WORLD_EXT}"))
}

/// The record's path for a save named `path` (whatever extension it was
/// given).
fn record_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let stem = name
        .strip_suffix(&format!(".{STALE_WORLD_EXT}"))
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
            pmc: Some(PmcSave {
                cash: 125_000.0,
                fuel: 240.0,
                stock: Vec::new(),
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

    /// An old TOML save as builds before #781 wrote it, world part and all.
    const OLD: &str = r#"
version = 1
hero = "mattias"
killed = [7, 9]
scripts = "{[\"nTimeElapsed\"]=18,}"

[[vehicles]]
model = 50550110
position = [460.0, -32.0, -1940.0]
yaw = 0.5
wrecked = true

[player]
position = [1.0, 2.0, 3.0]
facing = 0.0
yaw = 0.0
pitch = 0.0
hp = 64.0
weapon = "rpg"

[pmc]
cash = 125000.0
fuel = 240.0
stock = [["clusterbomb", 2]]
"#;

    #[test]
    fn round_trips_through_the_record() {
        let s = save();
        let bytes = s.to_record(1_795_000_000).unwrap();
        assert_eq!(bytes.len(), 0x3458);
        let back = SaveGame::from_record(&bytes).unwrap();
        let mut want = s.clone();
        want.info.cash = 125_000;
        want.info.fuel = 240;
        want.info.saved_at = 1_795_000_000;
        want.info.character = 3;
        assert_eq!(back, want);
    }

    /// An old save's story data loads; its world part does not.
    #[test]
    fn old_toml_saves_keep_what_the_record_carries() {
        let s = SaveGame::from_toml(OLD).unwrap();
        assert_eq!(s.hero, Some(Hero::Mattias));
        assert_eq!(s.scripts.as_deref(), Some("{[\"nTimeElapsed\"]=18,}"));
        let pmc = s.pmc.unwrap();
        assert_eq!((pmc.cash, pmc.fuel), (125_000.0, 240.0));
        assert_eq!(pmc.stock, [("clusterbomb".to_owned(), 2)]);
        // From before money and heroes: none kept.
        let bare = SaveGame::from_toml("version = 1\n").unwrap();
        assert_eq!((bare.pmc, bare.scripts, bare.hero), (None, None, None));
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
    fn saving_writes_the_record_alone_and_clears_old_files() {
        let d = dir("migrate");
        let slot = d.join("m2-alpha-save3.sav");
        std::fs::write(d.join("m2-alpha-save3.toml"), OLD).unwrap();
        std::fs::write(d.join("m2-alpha-save3.world.toml"), "version = 1\n").unwrap();
        assert_eq!(SaveGame::found(&slot), Some(d.join("m2-alpha-save3.toml")));
        let read = SaveGame::read(&slot).unwrap();
        assert_eq!(read.hero, Some(Hero::Mattias));
        read.write(&slot).unwrap();
        assert!(!d.join("m2-alpha-save3.toml").exists());
        assert!(!d.join("m2-alpha-save3.world.toml").exists());
        let files: Vec<_> = std::fs::read_dir(&d).unwrap().collect();
        assert_eq!(files.len(), 1, "only the record");
        let again = SaveGame::read(&slot).unwrap();
        assert_eq!(again.scripts, read.scripts);
        assert_eq!(again.hero, read.hero);
        assert!(again.info.saved_at > 0);
        // A path given as the old file's still finds the record.
        assert_eq!(
            SaveGame::read(&d.join("m2-alpha-save3.toml")).unwrap(),
            again
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A world file left beside a record by #781-#783 is never read.
    #[test]
    fn a_stale_world_file_is_ignored() {
        let d = dir("stale");
        let slot = d.join("m2-alpha-save4.sav");
        std::fs::write(&slot, save().to_record(5).unwrap()).unwrap();
        std::fs::write(d.join("m2-alpha-save4.world.toml"), "not toml [").unwrap();
        let s = SaveGame::read(&slot).unwrap();
        assert_eq!(s.hero, Some(Hero::Jennifer));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn other_versions_are_refused() {
        let mut s = save();
        s.version = VERSION + 1;
        assert!(SaveGame::from_toml(&s.to_toml()).is_err());
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
