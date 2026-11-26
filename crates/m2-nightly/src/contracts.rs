//! The contracts to play, read from the user's own `data\vz.wad` at run
//! time, and the night's order.
//!
//! Each contract is a script module named for its faction, `Con` and a
//! three-digit number (`PirCon001`); the archive keys scripts by
//! [`name_hash`] of the module name. So the contracts are the names of that
//! shape whose hash is a script in the archive's index (only the index is
//! read, no block is unpacked). From samples: the faction prefixes are
//! those of the story's contract ids (the contract-sweep list of earlier
//! autoplay rounds); a prefix not listed here would be missed.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::Context;
use m2_assets::lua::SCRIPT_TYPE;
use m2_assets::{WadIndex, name_hash};

/// Faction prefixes in story order: the first contract (`VzaCon001`), the
/// PMC's own, then the factions.
pub const FACTIONS: [&str; 9] = [
    "Vza", "Pmc", "Mec", "Jet", "Oil", "Gur", "Pir", "All", "Chi",
];

/// The story's first contract: it plays from the start, without
/// `--skip-to`.
pub const FIRST: &str = "VzaCon001";

/// Script names hashed in `index`.
fn script_hashes(index: &WadIndex) -> HashSet<u32> {
    index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == SCRIPT_TYPE)
        .map(|a| a.name_hash)
        .collect()
}

/// The ids of shape `<faction><kind><000..999>` that are scripts, in
/// faction order then number. `kind` is `Con` for contracts, `Job` for
/// jobs.
pub fn ids_in(scripts: &HashSet<u32>, kind: &str) -> Vec<String> {
    let mut out = Vec::new();
    for f in FACTIONS {
        for n in 0..1000 {
            let id = format!("{f}{kind}{n:03}");
            if scripts.contains(&name_hash(&id)) {
                out.push(id);
            }
        }
    }
    out
}

/// The contracts (and with `jobs`, the jobs after them) in the install.
pub fn from_install(install: &Path, jobs: bool) -> anyhow::Result<Vec<String>> {
    let wad = install.join("data").join("vz.wad");
    let mut f = BufReader::new(File::open(&wad).with_context(|| format!("{}", wad.display()))?);
    let index = WadIndex::read_from(&mut f).with_context(|| format!("{}", wad.display()))?;
    let scripts = script_hashes(&index);
    let mut ids = ids_in(&scripts, "Con");
    if jobs {
        ids.extend(ids_in(&scripts, "Job"));
    }
    Ok(ids)
}

/// The night's order: contracts never played first (in story order), then
/// the rest from the longest ago played, so nights that can't fit every
/// contract take turns through them. `last_played` maps an id to the unix
/// time it last ran.
pub fn schedule(ids: &[String], last_played: &BTreeMap<String, u64>) -> Vec<String> {
    let mut order: Vec<(u64, usize, &String)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (last_played.get(id).copied().unwrap_or(0), i, id))
        .collect();
    order.sort();
    order.into_iter().map(|(_, _, id)| id.clone()).collect()
}

/// The m2-alpha arguments that play one contract by autoplay.
pub fn alpha_args(id: &str, install: &Path, picture: &Path, mission_seconds: u32) -> Vec<String> {
    let mut a = vec![
        "--install".to_string(),
        install.display().to_string(),
        "--mission".into(),
        "--autoplay".into(),
        "--contracts".into(),
        "1".into(),
        "--mission-seconds".into(),
        mission_seconds.to_string(),
        "--size".into(),
        "640x360".into(),
        "--screenshot".into(),
        picture.display().to_string(),
    ];
    if id != FIRST {
        a.push("--skip-to".into());
        a.push(id.into());
    }
    a
}
