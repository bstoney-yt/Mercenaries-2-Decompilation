//! The game's save slots as `m2-alpha` keeps them: slot 0 the scripts'
//! autosave (`m2-alpha-autosave.toml`), 1 to [`SLOTS`] the player's
//! (`m2-alpha-save<n>.toml`), beside its settings file. The launcher reads
//! what each says (who, her cash and fuel, when it was written), copies
//! one into the first free slot and deletes one, as the game's own save
//! screens do.
//!
//! # Provenance
//!
//! The file names and slot numbers are `m2-alpha`'s (its `save.rs` and
//! pause menu). That file format is the port's own: the 360's save record
//! is not written yet, so neither the game's own play time nor its
//! percent complete is in a save, and the list shows neither.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

/// The player's slots (the pause menu's save screen goes to 20).
pub const SLOTS: u32 = 20;
/// The save version `m2-alpha` reads (`save::VERSION`).
pub const VERSION: u32 = 1;

/// Slot `n`'s file name.
pub fn file_name(n: u32) -> String {
    if n == 0 {
        "m2-alpha-autosave.toml".to_owned()
    } else {
        format!("m2-alpha-save{n}.toml")
    }
}

/// What the launcher reads of a save.
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
    pub written: Option<SystemTime>,
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
}

/// Reads slot `n` in `dir`, if it holds a save.
pub fn read(dir: &Path, n: u32) -> Option<Slot> {
    let path = dir.join(file_name(n));
    let written = std::fs::metadata(&path).ok()?.modified().ok();
    let mut slot = Slot {
        number: n,
        path: path.clone(),
        hero: None,
        cash: 0.0,
        fuel: 0.0,
        written,
        problem: None,
    };
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| toml::from_str::<Raw>(&t).map_err(|e| e.message().to_owned()))
    {
        Ok(raw) => {
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
        }
        Err(e) => slot.problem = Some(e),
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
    (1..=SLOTS).find(|&n| !dir.join(file_name(n)).exists())
}

/// Copies slot `from` into the first free slot: the one it went to.
pub fn copy(dir: &Path, from: u32) -> Result<u32, String> {
    let to = free_slot(dir).ok_or("every slot is full")?;
    std::fs::copy(dir.join(file_name(from)), dir.join(file_name(to))).map_err(|e| e.to_string())?;
    Ok(to)
}

pub fn delete(dir: &Path, n: u32) -> Result<(), String> {
    std::fs::remove_file(dir.join(file_name(n))).map_err(|e| e.to_string())
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
    use std::time::Duration;

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

    #[test]
    fn slots_are_the_games_file_names() {
        assert_eq!(file_name(0), "m2-alpha-autosave.toml");
        assert_eq!(file_name(3), "m2-alpha-save3.toml");
    }

    #[test]
    fn a_save_is_read_as_the_game_wrote_it() {
        let d = dir("read");
        std::fs::write(d.join(file_name(2)), SAVE).unwrap();
        let s = read(&d, 2).unwrap();
        assert_eq!(s.hero.as_deref(), Some("mattias"));
        assert_eq!(
            (s.cash, s.fuel, s.problem.clone()),
            (125_000.0, 240.0, None)
        );
        assert_eq!(s.title(), "Save 2");
        assert!(read(&d, 3).is_none());
        // Saves from before money and heroes have neither.
        std::fs::write(
            d.join(file_name(0)),
            "version = 1\nkilled = []\nvehicles = []\n",
        )
        .unwrap();
        let auto = read(&d, 0).unwrap();
        assert_eq!((auto.hero, auto.cash, auto.problem), (None, 0.0, None));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn another_builds_save_says_why_it_wont_load() {
        let d = dir("version");
        std::fs::write(
            d.join(file_name(1)),
            SAVE.replace("version = 1", "version = 9"),
        )
        .unwrap();
        assert!(read(&d, 1).unwrap().problem.unwrap().contains("version 9"));
        std::fs::write(d.join(file_name(1)), "not toml [").unwrap();
        assert!(read(&d, 1).unwrap().problem.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copying_takes_the_first_free_slot_and_deleting_frees_it() {
        let d = dir("copy");
        for n in [0, 1, 2, 4] {
            std::fs::write(d.join(file_name(n)), SAVE).unwrap();
        }
        assert_eq!(copy(&d, 0), Ok(3));
        assert_eq!(copy(&d, 1), Ok(5));
        assert_eq!(scan(&d).len(), 6);
        delete(&d, 3).unwrap();
        assert_eq!(free_slot(&d), Some(3));
        for n in 1..=SLOTS {
            std::fs::write(d.join(file_name(n)), SAVE).unwrap();
        }
        assert_eq!(copy(&d, 0), Err("every slot is full".to_owned()));
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
