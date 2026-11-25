//! The game's save slots as `m2-alpha` keeps them: slot 0 the scripts'
//! autosave (`m2-alpha-autosave.sav`), 1 to [`SLOTS`] the player's
//! (`m2-alpha-save<n>.sav`), beside its settings file. The launcher reads
//! what each says (who, her cash and fuel, the time played, the last
//! contract, the percent done, when it was saved), copies one into the
//! first free slot and deletes one, as the game's own save screens do.
//!
//! # Provenance
//!
//! Each `.sav` is one 360 save record (**from the 360 code**; laid out
//! and traced in `m2_game::save_record`): big-endian, 0x3458 bytes, its
//! version word 4 and size word 0x3458 first, then the owner's id. The
//! launcher reads only the fields the 360's save list shows
//! (`addSaveGame`, `0x8237E2F0`), at their offsets in the file: seconds
//! played +0x10, cash +0x14, fuel +0x18, the time stamp +0x20 (8 bytes),
//! the contract +0x28 (0x20 bytes), the character +0x49 (1 Mattias,
//! 2 Chris, 3 Jennifer), the autosave flag +0x4C and the percent +0x4D;
//! the play time shown as `%d:%02d:%02d`.
//!
//! **Invented**: the file names and slot numbers (`m2-alpha`'s `save.rs`
//! and pause menu), and the `.world.toml` beside each record (the port's
//! own, copied and deleted with it). Saves of the port's older TOML
//! format (`m2-alpha-save<n>.toml`) are still listed, with no play time,
//! contract or percent (they had none).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

/// The player's slots (the pause menu's save screen goes to 20).
pub const SLOTS: u32 = 20;
/// The old TOML saves' version `m2-alpha` reads (`save::VERSION`).
pub const VERSION: u32 = 1;
/// The 360's record: version, size, and its length in a file.
pub const RECORD_VERSION: u32 = 4;
pub const RECORD_SIZE: u32 = 0x3458;
/// The owner's id `m2-alpha` writes (`save_record::OWNER`, invented).
pub const OWNER: u32 = 0;

/// The extensions of a slot's files: the record, the port's world file
/// beside it, and the old TOML save.
const RECORD: &str = "sav";
const WORLD: &str = "world.toml";
const LEGACY: &str = "toml";

/// Slot `n`'s file name, without its extension.
fn stem(n: u32) -> String {
    if n == 0 {
        "m2-alpha-autosave".to_owned()
    } else {
        format!("m2-alpha-save{n}")
    }
}

/// Slot `n`'s record's file name.
#[cfg(test)]
fn file_name(n: u32) -> String {
    format!("{}.{RECORD}", stem(n))
}

/// Slot `n`'s files in `dir`: the record, its world file, the old save.
fn files(dir: &Path, n: u32) -> [PathBuf; 3] {
    let s = stem(n);
    [RECORD, WORLD, LEGACY].map(|e| dir.join(format!("{s}.{e}")))
}

/// The file slot `n` in `dir` holds its save in: the record, else an old
/// TOML save.
fn save_file(dir: &Path, n: u32) -> Option<PathBuf> {
    let [record, _, legacy] = files(dir, n);
    [record, legacy].into_iter().find(|p| p.is_file())
}

/// What the launcher reads of an old TOML save.
#[derive(Debug, Deserialize)]
struct Raw {
    version: u32,
    #[serde(default)]
    hero: Option<String>,
    #[serde(default)]
    pmc: Option<RawPmc>,
}

#[derive(Debug, Deserialize)]
struct RawPmc {
    cash: f64,
    fuel: f64,
}

/// A slot with a save in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub number: u32,
    pub path: PathBuf,
    /// Whose it is (`mattias`, `jennifer`, `chris`); none in saves from
    /// before heroes.
    pub hero: Option<String>,
    pub cash: f64,
    pub fuel: f64,
    /// When it was saved: the record's stamp, else the file's time.
    pub written: Option<SystemTime>,
    /// Seconds played, the last contract and percent done, from the
    /// record (none in an old TOML save).
    pub played: Option<u32>,
    pub contract: String,
    pub percent: Option<u8>,
    /// The record says it was the scripts' autosave.
    pub autosave: bool,
    /// Why the game can't load it, if it can't.
    pub problem: Option<String>,
}

impl Slot {
    pub fn title(&self) -> String {
        if self.number == 0 {
            "Autosave".to_owned()
        } else {
            format!("Save {}", self.number)
        }
    }

    /// The play time as the 360's save list shows it, `H:MM:SS`.
    pub fn played_text(&self) -> Option<String> {
        self.played
            .map(|s| format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60))
    }
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The save list's fields of a 360 record's bytes, into `slot`.
fn read_record(bytes: &[u8], slot: &mut Slot) -> Result<(), String> {
    if bytes.len() != RECORD_SIZE as usize {
        return Err(format!(
            "{} bytes, not a save record's {RECORD_SIZE}",
            bytes.len()
        ));
    }
    let (version, size, owner) = (be32(bytes, 0), be32(bytes, 4), be32(bytes, 8));
    if (version, size, owner) != (RECORD_VERSION, RECORD_SIZE, OWNER) {
        return Err(format!(
            "made by another build (record version {version}, size {size:#x}, owner {owner:#x})"
        ));
    }
    slot.played = Some(be32(bytes, 0x10));
    slot.cash = f64::from(be32(bytes, 0x14));
    slot.fuel = f64::from(be32(bytes, 0x18) as i32);
    let stamp = u64::from_be_bytes(bytes[0x20..0x28].try_into().expect("8 bytes"));
    if stamp > 0 {
        slot.written = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(stamp));
    }
    let contract = &bytes[0x28..0x48];
    let end = contract
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(contract.len());
    slot.contract = String::from_utf8_lossy(&contract[..end]).into_owned();
    slot.hero = match bytes[0x49] {
        1 => Some("mattias".to_owned()),
        2 => Some("chris".to_owned()),
        3 => Some("jennifer".to_owned()),
        _ => None,
    };
    slot.autosave = bytes[0x4C] != 0;
    slot.percent = Some(bytes[0x4D].min(100));
    Ok(())
}

/// Reads slot `n` in `dir`, if it holds a save.
pub fn read(dir: &Path, n: u32) -> Option<Slot> {
    let path = save_file(dir, n)?;
    let written = std::fs::metadata(&path).ok()?.modified().ok();
    let mut slot = Slot {
        number: n,
        path: path.clone(),
        hero: None,
        cash: 0.0,
        fuel: 0.0,
        written,
        played: None,
        contract: String::new(),
        percent: None,
        autosave: false,
        problem: None,
    };
    let legacy = path.extension().is_some_and(|e| e == LEGACY);
    let read = if legacy {
        std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str::<Raw>(&t).map_err(|e| e.message().to_owned()))
            .map(|raw| {
                if raw.version != VERSION {
                    slot.problem = Some(format!(
                        "made by another build (version {}, this one reads {VERSION})",
                        raw.version
                    ));
                }
                slot.hero = raw.hero;
                if let Some(p) = raw.pmc {
                    slot.cash = p.cash;
                    slot.fuel = p.fuel;
                }
            })
    } else {
        std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| read_record(&b, &mut slot))
    };
    if let Err(e) = read {
        slot.problem = Some(e);
    }
    Some(slot)
}

/// Every slot with a save, newest first.
pub fn scan(dir: &Path) -> Vec<Slot> {
    let mut slots: Vec<Slot> = (0..=SLOTS).filter_map(|n| read(dir, n)).collect();
    slots.sort_by(|a, b| b.written.cmp(&a.written).then(a.number.cmp(&b.number)));
    slots
}

/// The first of the player's slots with nothing in it.
pub fn free_slot(dir: &Path) -> Option<u32> {
    (1..=SLOTS).find(|&n| save_file(dir, n).is_none())
}

/// Copies slot `from` into the first free slot (each of its files): the
/// one it went to.
pub fn copy(dir: &Path, from: u32) -> Result<u32, String> {
    let to = free_slot(dir).ok_or("every slot is full")?;
    save_file(dir, from).ok_or("that slot is empty")?;
    for (a, b) in files(dir, from).into_iter().zip(files(dir, to)) {
        if a.is_file() {
            std::fs::copy(a, b).map_err(|e| e.to_string())?;
        }
    }
    Ok(to)
}

/// Deletes slot `n`'s files.
pub fn delete(dir: &Path, n: u32) -> Result<(), String> {
    save_file(dir, n).ok_or("that slot is empty")?;
    for f in files(dir, n) {
        if f.is_file() {
            std::fs::remove_file(f).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// How long ago `then` was, as the launcher's list says it.
pub fn ago(now: SystemTime, then: SystemTime) -> String {
    let secs = now.duration_since(then).map_or(0, |d| d.as_secs());
    let (n, unit) = match secs {
        0..60 => return "just now".to_owned(),
        60..3600 => (secs / 60, "minute"),
        3600..86_400 => (secs / 3600, "hour"),
        86_400..2_592_000 => (secs / 86_400, "day"),
        2_592_000..31_536_000 => (secs / 2_592_000, "month"),
        _ => (secs / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// Cash as the game shows it: `$1,250,000`.
pub fn money(cash: f64) -> String {
    let whole = cash.max(0.0).round() as u64;
    let digits = whole.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    out.push('$');
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A hero's name as the menus show it.
pub fn hero_name(hero: &str) -> &'static str {
    match hero.to_ascii_lowercase().as_str() {
        "mattias" => "Mattias Nilsson",
        "chris" => "Chris Jacobs",
        _ => "Jennifer Mui",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("m2-launcher-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const SAVE: &str = r#"
version = 1
hero = "mattias"
killed = []
vehicles = []

[player]
position = [1.0, 2.0, 3.0]
facing = 0.0
yaw = 0.0
pitch = 0.0
hp = 100.0
weapon = "rifle"

[pmc]
cash = 125000.0
fuel = 240.0
stock = []
"#;

    /// A record as `m2-alpha` writes one: 3:07:09 played, $1,250,000,
    /// 240 fuel, `VzaCon001`, Chris, the autosave, 12 percent.
    fn record() -> Vec<u8> {
        let mut b = vec![0u8; RECORD_SIZE as usize];
        b[0..4].copy_from_slice(&RECORD_VERSION.to_be_bytes());
        b[4..8].copy_from_slice(&RECORD_SIZE.to_be_bytes());
        b[0x10..0x14].copy_from_slice(&11_229u32.to_be_bytes());
        b[0x14..0x18].copy_from_slice(&1_250_000u32.to_be_bytes());
        b[0x18..0x1C].copy_from_slice(&240u32.to_be_bytes());
        b[0x20..0x28].copy_from_slice(&1_000_000_000u64.to_be_bytes());
        b[0x28..0x31].copy_from_slice(b"VzaCon001");
        b[0x49] = 2;
        b[0x4C] = 1;
        b[0x4D] = 12;
        b
    }

    fn old_name(n: u32) -> String {
        format!("{}.toml", stem(n))
    }

    #[test]
    fn slots_are_the_games_file_names() {
        assert_eq!(file_name(0), "m2-alpha-autosave.sav");
        assert_eq!(file_name(3), "m2-alpha-save3.sav");
    }

    #[test]
    fn a_record_is_read_as_the_360s_list_shows_it() {
        let d = dir("record");
        std::fs::write(d.join(file_name(4)), record()).unwrap();
        let s = read(&d, 4).unwrap();
        assert_eq!(s.problem, None);
        assert_eq!(s.hero.as_deref(), Some("chris"));
        assert_eq!((s.cash, s.fuel), (1_250_000.0, 240.0));
        assert_eq!(s.played_text().as_deref(), Some("3:07:09"));
        assert_eq!(
            (s.contract.as_str(), s.percent, s.autosave),
            ("VzaCon001", Some(12), true)
        );
        assert_eq!(
            s.written,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        );
        // Another version, or a file cut short, says why.
        let mut other = record();
        other[3] = 5;
        std::fs::write(d.join(file_name(4)), other).unwrap();
        assert!(read(&d, 4).unwrap().problem.unwrap().contains("version 5"));
        std::fs::write(d.join(file_name(4)), &record()[..100]).unwrap();
        assert!(read(&d, 4).unwrap().problem.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_old_save_is_read_as_the_game_wrote_it() {
        let d = dir("read");
        std::fs::write(d.join(old_name(2)), SAVE).unwrap();
        let s = read(&d, 2).unwrap();
        assert_eq!(s.hero.as_deref(), Some("mattias"));
        assert_eq!(
            (s.cash, s.fuel, s.problem.clone(), s.played),
            (125_000.0, 240.0, None, None)
        );
        assert_eq!(s.title(), "Save 2");
        assert!(read(&d, 3).is_none());
        // Saves from before money and heroes have neither.
        std::fs::write(
            d.join(old_name(0)),
            "version = 1\nkilled = []\nvehicles = []\n",
        )
        .unwrap();
        let auto = read(&d, 0).unwrap();
        assert_eq!((auto.hero, auto.cash, auto.problem), (None, 0.0, None));
        // The record wins over an old save in the same slot.
        std::fs::write(d.join(file_name(2)), record()).unwrap();
        assert_eq!(read(&d, 2).unwrap().hero.as_deref(), Some("chris"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn another_builds_save_says_why_it_wont_load() {
        let d = dir("version");
        std::fs::write(
            d.join(old_name(1)),
            SAVE.replace("version = 1", "version = 9"),
        )
        .unwrap();
        assert!(read(&d, 1).unwrap().problem.unwrap().contains("version 9"));
        std::fs::write(d.join(old_name(1)), "not toml [").unwrap();
        assert!(read(&d, 1).unwrap().problem.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copying_takes_the_first_free_slot_and_deleting_frees_it() {
        let d = dir("copy");
        for n in [0, 1, 2, 4] {
            std::fs::write(d.join(file_name(n)), record()).unwrap();
            std::fs::write(d.join(format!("{}.world.toml", stem(n))), "version = 1\n").unwrap();
        }
        assert_eq!(copy(&d, 0), Ok(3));
        assert!(d.join("m2-alpha-save3.world.toml").is_file());
        assert_eq!(copy(&d, 1), Ok(5));
        assert_eq!(scan(&d).len(), 6);
        delete(&d, 3).unwrap();
        assert!(!d.join("m2-alpha-save3.world.toml").exists());
        assert_eq!(free_slot(&d), Some(3));
        // An old save fills its slot too.
        std::fs::write(d.join(old_name(3)), SAVE).unwrap();
        assert_eq!(free_slot(&d), Some(6));
        for n in 1..=SLOTS {
            std::fs::write(d.join(file_name(n)), record()).unwrap();
        }
        assert_eq!(copy(&d, 0), Err("every slot is full".to_owned()));
        assert!(delete(&d, 99).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn times_and_money_read_as_the_list_shows_them() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let at = |s| t + Duration::from_secs(s);
        assert_eq!(ago(at(5), t), "just now");
        assert_eq!(ago(at(60), t), "1 minute ago");
        assert_eq!(ago(at(7200), t), "2 hours ago");
        assert_eq!(ago(at(86_400 * 3), t), "3 days ago");
        assert_eq!(ago(t, at(10)), "just now");
        assert_eq!(money(0.0), "$0");
        assert_eq!(money(999.0), "$999");
        assert_eq!(money(1000.0), "$1,000");
        assert_eq!(money(1_250_000.4), "$1,250,000");
    }
}
