//! Which build of the game to play: the one installed beside the launcher,
//! the one kept for rolling back (`update\previous\`, [`crate::update`]),
//! and every build kept under `builds\<tag>\` beside the launcher, newest
//! first, with the latest release offered for download when it is none of
//! those. The choice is pinned in the launcher's settings
//! ([`crate::launch::Settings::build`]).
//!
//! Nothing here deletes a build: the update swaps only ever replace
//! `update\previous\`, so the build there is copied into `builds\` first
//! ([`keep_previous`]) and stays pickable. A pinned build whose program is
//! gone falls back to the installed one, and says so.
//!
//! Everything here is the launcher's own (**invented**); nothing is from
//! the game. A kept build runs from the launcher's folder, so it reads the
//! same settings and saves as the installed one.

use std::path::{Path, PathBuf};

use crate::update::{MANIFEST, Manifest, UPDATE_DIR, sha256_hex, short};

/// The folder beside the launcher that keeps builds, one per tag.
pub const BUILDS_DIR: &str = "builds";
/// The game's program in a build.
pub const GAME: &str = "m2-alpha.exe";

/// Where a build comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Beside the launcher: what Play starts unless another is pinned.
    Installed,
    /// Kept for rolling back.
    Previous,
    /// Kept under `builds\`.
    Kept,
    /// The latest release, not on this computer yet.
    Latest,
}

/// One build on offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// What the settings pin: `installed`, or a build's tag.
    pub key: String,
    pub tag: String,
    /// The commit, when known.
    pub sha: String,
    /// When it was built (UTC, as its manifest says), if known.
    pub built: String,
    pub kind: Kind,
    /// Its program; `None` for a release not downloaded.
    pub exe: Option<PathBuf>,
}

impl Choice {
    /// The line the picker shows: the tag (our own release title), the
    /// build's date, and where it is.
    pub fn label(&self) -> String {
        let date = self.built.get(..10).unwrap_or("");
        let date = if date.is_empty() {
            String::new()
        } else {
            format!(" - {date}")
        };
        let what = match self.kind {
            Kind::Installed => " (installed)",
            Kind::Previous => " (kept for rollback)",
            Kind::Kept => "",
            Kind::Latest => " (latest release, not downloaded)",
        };
        format!("{}{date}{what}", self.tag)
    }
}

pub const INSTALLED: &str = "installed";

fn read_manifest(dir: &Path) -> Option<Manifest> {
    Manifest::parse(&std::fs::read_to_string(dir.join(MANIFEST)).ok()?).ok()
}

/// The builds on offer in the launcher's folder `dir`: the installed one
/// first (its commit `current`, if known), then the rollback build and the
/// kept ones, newest built first, one per commit; then `latest` if it is
/// none of them.
pub fn list(dir: &Path, current: Option<&str>, latest: Option<&Manifest>) -> Vec<Choice> {
    let mut out = vec![Choice {
        key: INSTALLED.to_owned(),
        tag: current.map_or_else(
            || "this build".to_owned(),
            |c| format!("build {}", short(c)),
        ),
        sha: current.unwrap_or("").to_owned(),
        built: String::new(),
        kind: Kind::Installed,
        exe: Some(dir.join(GAME)),
    }];
    let mut others = Vec::new();
    let previous = dir.join(UPDATE_DIR).join("previous");
    if let Some(m) = read_manifest(&previous)
        && previous.join(GAME).is_file()
    {
        others.push((m, Kind::Previous, previous.join(GAME)));
    }
    if let Ok(read) = std::fs::read_dir(dir.join(BUILDS_DIR)) {
        for e in read.flatten() {
            let d = e.path();
            if let Some(m) = read_manifest(&d)
                && d.join(GAME).is_file()
            {
                others.push((m, Kind::Kept, d.join(GAME)));
            }
        }
    }
    others.sort_by(|a, b| b.0.built.cmp(&a.0.built).then(a.0.tag.cmp(&b.0.tag)));
    let same = |a: &str, b: &str| !a.is_empty() && a.eq_ignore_ascii_case(b);
    for (m, kind, exe) in others {
        if out.iter().any(|c| same(&c.sha, &m.sha) || c.key == m.tag) {
            continue;
        }
        out.push(Choice {
            key: m.tag.clone(),
            tag: m.tag,
            sha: m.sha,
            built: m.built,
            kind,
            exe: Some(exe),
        });
    }
    if let Some(m) = latest
        && !out.iter().any(|c| same(&c.sha, &m.sha))
    {
        out.push(Choice {
            key: m.tag.clone(),
            tag: m.tag.clone(),
            sha: m.sha.clone(),
            built: m.built.clone(),
            kind: Kind::Latest,
            exe: None,
        });
    }
    out
}

/// What Play starts for the pinned choice `pinned` (`None` or
/// [`INSTALLED`]: the installed build): the choice, and a note when the
/// pinned build is not here (missing, or a release not downloaded), in
/// which case the installed build is started.
pub fn resolve(choices: &[Choice], pinned: Option<&str>) -> (Choice, Option<String>) {
    let installed = choices
        .iter()
        .find(|c| c.kind == Kind::Installed)
        .cloned()
        .expect("the installed build is always listed");
    let Some(key) = pinned.filter(|k| *k != INSTALLED) else {
        return (installed, None);
    };
    match choices.iter().find(|c| c.key == key) {
        Some(c) if c.exe.as_ref().is_some_and(|e| e.is_file()) => (c.clone(), None),
        Some(c) if c.kind == Kind::Latest => (
            installed,
            Some(format!(
                "{} is not downloaded yet: playing the installed build.",
                c.tag
            )),
        ),
        _ => (
            installed,
            Some(format!(
                "The pinned build {key} is no longer here: playing the installed build."
            )),
        ),
    }
}

/// Copies the rollback build (`update\previous\`) into `builds\<tag>\` if
/// it is not kept there yet, so the next update's swap (which replaces
/// `update\previous\`) leaves it pickable. Never deletes anything. The tag
/// kept comes back.
pub fn keep_previous(dir: &Path) -> Result<Option<String>, String> {
    let previous = dir.join(UPDATE_DIR).join("previous");
    let Some(m) = read_manifest(&previous) else {
        return Ok(None);
    };
    if !previous.join(GAME).is_file() || !safe_tag(&m.tag) {
        return Ok(None);
    }
    let to = dir.join(BUILDS_DIR).join(&m.tag);
    if to.join(GAME).is_file() {
        return Ok(None);
    }
    std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    std::fs::copy(previous.join(GAME), to.join(GAME)).map_err(|e| e.to_string())?;
    let mut kept = m.clone();
    kept.files.retain(|f| f.name == GAME);
    std::fs::write(to.join(MANIFEST), kept.to_text()).map_err(|e| e.to_string())?;
    Ok(Some(m.tag))
}

/// Keeps release `m`'s game program under `builds\<tag>\`, fetched by
/// `fetch` and checked against the manifest's size and SHA-256 (the
/// launcher itself is not taken: it stays the installed one).
pub fn keep_release(
    dir: &Path,
    m: &Manifest,
    fetch: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    if !safe_tag(&m.tag) {
        return Err(format!("{}: not a build tag", m.tag));
    }
    let f = m
        .files
        .iter()
        .find(|f| f.name == GAME)
        .ok_or_else(|| format!("{}: no {GAME} in it", m.tag))?;
    let bytes = fetch(GAME)?;
    if bytes.len() as u64 != f.size || sha256_hex(&bytes) != f.sha256.to_ascii_lowercase() {
        return Err(format!("{GAME}: the download does not match its checksum"));
    }
    let to = dir.join(BUILDS_DIR).join(&m.tag);
    std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    std::fs::write(to.join(GAME), &bytes).map_err(|e| e.to_string())?;
    let mut kept = m.clone();
    kept.files.retain(|f| f.name == GAME);
    std::fs::write(to.join(MANIFEST), kept.to_text()).map_err(|e| e.to_string())?;
    Ok(to.join(GAME))
}

/// A tag fit for a folder name (`build-<date>-<sha>`).
fn safe_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !tag.starts_with('.')
}

/// Starts `game` with `args` from the launcher's folder `home` (where the
/// game finds its settings and saves, whichever build it is).
pub fn spawn(game: &Path, args: &[String], home: &Path) -> Result<std::process::Child, String> {
    std::process::Command::new(game)
        .args(args)
        .current_dir(home)
        .spawn()
        .map_err(|e| format!("{}: {e}", game.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::FileEntry;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("m2-builds-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn manifest(sha: &str, built: &str, game: &[u8]) -> Manifest {
        Manifest {
            tag: format!("build-{}-{}", built[..10].replace('-', ""), short(sha)),
            sha: sha.to_owned(),
            built: built.to_owned(),
            files: vec![FileEntry {
                name: GAME.to_owned(),
                size: game.len() as u64,
                sha256: sha256_hex(game),
            }],
        }
    }

    /// A fake store: the installed build, a rollback build and kept ones.
    fn store(name: &str) -> PathBuf {
        let d = dir(name);
        std::fs::write(d.join(GAME), b"installed").unwrap();
        let prev = d.join(UPDATE_DIR).join("previous");
        std::fs::create_dir_all(&prev).unwrap();
        std::fs::write(prev.join(GAME), b"prev").unwrap();
        let m = manifest("bbbbbbbbbbbb", "2026-11-24T02:00:00Z", b"prev");
        std::fs::write(prev.join(MANIFEST), m.to_text()).unwrap();
        for (sha, built, body) in [
            ("cccccccccccc", "2026-11-20T02:00:00Z", b"old" as &[u8]),
            ("dddddddddddd", "2026-11-25T02:00:00Z", b"newer"),
        ] {
            let m = manifest(sha, built, body);
            let k = d.join(BUILDS_DIR).join(&m.tag);
            std::fs::create_dir_all(&k).unwrap();
            std::fs::write(k.join(GAME), body).unwrap();
            std::fs::write(k.join(MANIFEST), m.to_text()).unwrap();
        }
        // A folder without its program is not offered.
        std::fs::create_dir_all(d.join(BUILDS_DIR).join("broken")).unwrap();
        d
    }

    #[test]
    fn the_installed_build_comes_first_then_the_newest() {
        let d = store("list");
        let latest = manifest("eeeeeeeeeeee", "2026-11-26T02:00:00Z", b"latest");
        let c = list(&d, Some("aaaaaaaaaaaa"), Some(&latest));
        let kinds: Vec<(Kind, &str)> = c.iter().map(|c| (c.kind, c.built.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (Kind::Installed, ""),
                (Kind::Kept, "2026-11-25T02:00:00Z"),
                (Kind::Previous, "2026-11-24T02:00:00Z"),
                (Kind::Kept, "2026-11-20T02:00:00Z"),
                (Kind::Latest, "2026-11-26T02:00:00Z"),
            ]
        );
        assert_eq!(c[0].label(), "build aaaaaaa (installed)");
        assert_eq!(
            c[2].label(),
            "build-20261124-bbbbbbb - 2026-11-24 (kept for rollback)"
        );
        assert_eq!(
            c[4].label(),
            "build-20261126-eeeeeee - 2026-11-26 (latest release, not downloaded)"
        );
        // The latest is not offered again once it is the installed build.
        let c = list(&d, Some("eeeeeeeeeeee"), Some(&latest));
        assert!(c.iter().all(|c| c.kind != Kind::Latest));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_pinned_build_starts_and_a_missing_one_falls_back() {
        let d = store("resolve");
        let c = list(&d, None, None);
        let (pick, note) = resolve(&c, None);
        assert_eq!((pick.kind, note), (Kind::Installed, None));
        let kept = c
            .iter()
            .find(|c| c.sha == "dddddddddddd")
            .unwrap()
            .key
            .clone();
        let (pick, note) = resolve(&c, Some(&kept));
        assert_eq!(pick.exe.unwrap(), d.join(BUILDS_DIR).join(&kept).join(GAME));
        assert!(note.is_none());
        // Its program gone: the installed build, with a note.
        std::fs::remove_file(d.join(BUILDS_DIR).join(&kept).join(GAME)).unwrap();
        let (pick, note) = resolve(&c, Some(&kept));
        assert_eq!(pick.kind, Kind::Installed);
        assert!(note.unwrap().contains("no longer here"));
        let (_, note) = resolve(&c, Some("build-never"));
        assert!(note.unwrap().contains("build-never"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_rollback_build_is_copied_to_be_kept_and_nothing_is_deleted() {
        let d = store("keep");
        let tag = keep_previous(&d).unwrap().unwrap();
        let kept = d.join(BUILDS_DIR).join(&tag);
        assert_eq!(std::fs::read(kept.join(GAME)).unwrap(), b"prev");
        // The rollback store is left as it was.
        assert!(d.join(UPDATE_DIR).join("previous").join(GAME).is_file());
        // Once kept, not again.
        assert_eq!(keep_previous(&d).unwrap(), None);
        // Every build is still there.
        assert_eq!(std::fs::read_dir(d.join(BUILDS_DIR)).unwrap().count(), 4);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_release_is_kept_only_if_it_checks_out() {
        let d = dir("release");
        let m = manifest("ffffffffffff", "2026-11-26T02:00:00Z", b"the game");
        let bad = keep_release(&d, &m, &mut |_| Ok(b"tampered".to_vec()));
        assert!(bad.unwrap_err().contains("checksum"));
        assert!(!d.join(BUILDS_DIR).join(&m.tag).join(GAME).exists());
        let exe = keep_release(&d, &m, &mut |name| {
            assert_eq!(name, GAME);
            Ok(b"the game".to_vec())
        })
        .unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"the game");
        let c = list(&d, None, Some(&m));
        assert!(c.iter().any(|c| c.kind == Kind::Kept && c.key == m.tag));
        assert!(c.iter().all(|c| c.kind != Kind::Latest));
        // A tag that isn't a folder name is refused.
        let mut evil = m.clone();
        evil.tag = "..\\escape".into();
        assert!(keep_release(&d, &evil, &mut |_| Ok(b"the game".to_vec())).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
