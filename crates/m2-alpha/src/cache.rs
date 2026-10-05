//! The catalogue of vehicle, person and weapon models, kept between runs.
//!
//! Finding them reads the whole world archive (12 to 14 seconds on a
//! desktop), so the result is written beside the settings file (or the exe) and reused
//! while the archive's size and modified time are unchanged. The cache
//! holds only asset hashes and the role names taken from texture names; it
//! lives on the player's machine and is never shipped.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use m2_engine::vehicle::{NAMED_WEAPONS, WEAPONS};
use m2_engine::{Catalogue, CatalogueEntry, PersonModel};
use serde::{Deserialize, Serialize};

/// Bumped whenever `catalogue()` changes what it finds, so old caches are
/// searched afresh.
const VERSION: u32 = 2;
const FILE_NAME: &str = "m2-alpha-catalogue.toml";

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Cached {
    version: u32,
    /// The archive's size in bytes and modified time (seconds since 1970).
    archive: (u64, u64),
    vehicles: Vec<Vehicle>,
    people: Vec<Person>,
    weapons: Vec<(String, u32)>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Vehicle {
    model: u32,
    kind: String,
    label: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Person {
    model: u32,
    role: String,
    soldier: bool,
}

/// Where the cache goes (see [`Config::file_beside`](crate::config::Config::file_beside)).
pub fn default_path() -> PathBuf {
    crate::config::Config::file_beside(FILE_NAME)
}

/// What identifies this version of `archive`: its size and modified time.
pub fn stamp(archive: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(archive).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_secs()))
}

/// The catalogue saved at `path` for the archive with `stamp`, if any.
pub fn read(path: &Path, stamp: (u64, u64)) -> Option<Catalogue> {
    let cached: Cached = toml::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if cached.version != VERSION || cached.archive != stamp {
        return None;
    }
    let weapons = cached
        .weapons
        .into_iter()
        .map(|(name, model)| {
            // Names are kept 'static by matching the known weapons.
            let &(known, _) = WEAPONS
                .iter()
                .chain(&NAMED_WEAPONS)
                .find(|(n, _)| *n == name)?;
            Some((known, model))
        })
        .collect::<Option<_>>()?;
    Some(Catalogue {
        vehicles: cached
            .vehicles
            .into_iter()
            .map(|v| CatalogueEntry {
                model: v.model,
                kind: v.kind,
                label: v.label,
            })
            .collect(),
        people: cached
            .people
            .into_iter()
            .map(|p| PersonModel {
                model: p.model,
                role: p.role,
                soldier: p.soldier,
            })
            .collect(),
        weapons,
    })
}

/// Saves `catalogue` for the archive with `stamp`.
pub fn write(path: &Path, stamp: (u64, u64), catalogue: &Catalogue) -> std::io::Result<()> {
    let cached = Cached {
        version: VERSION,
        archive: stamp,
        vehicles: catalogue
            .vehicles
            .iter()
            .map(|v| Vehicle {
                model: v.model,
                kind: v.kind.clone(),
                label: v.label.clone(),
            })
            .collect(),
        people: catalogue
            .people
            .iter()
            .map(|p| Person {
                model: p.model,
                role: p.role.clone(),
                soldier: p.soldier,
            })
            .collect(),
        weapons: catalogue
            .weapons
            .iter()
            .map(|&(n, m)| (n.to_owned(), m))
            .collect(),
    };
    let text = toml::to_string(&cached).expect("a catalogue always serialises");
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_while_the_archive_is_unchanged() {
        let catalogue = Catalogue {
            vehicles: vec![CatalogueEntry {
                model: 1,
                kind: "car".into(),
                label: "global_veh_hondacrx".into(),
            }],
            people: vec![PersonModel {
                model: 2,
                role: "vz_hum_soldier".into(),
                soldier: true,
            }],
            weapons: vec![("ak47", 3)],
        };
        let dir = std::env::temp_dir().join(format!("m2-alpha-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        write(&path, (10, 20), &catalogue).unwrap();
        assert_eq!(read(&path, (10, 20)), Some(catalogue));
        assert_eq!(read(&path, (10, 21)), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
