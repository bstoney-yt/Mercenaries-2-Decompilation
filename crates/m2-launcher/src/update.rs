//! Updating the play build from the project's GitHub Releases: finding
//! the newest build, downloading it, checking it, swapping it in beside
//! the launcher, and rolling back to the build before.
//!
//! Everything here is the launcher's own (**invented**); nothing is from
//! the game.
//!
//! # The releases
//!
//! The release workflow (`.github/workflows/release.yml`) publishes each
//! merge to `main` as a pre-release tagged `build-<date>-<sha7>`, with
//! [`FILES`] and a [`MANIFEST`]: the build's commit, tag, date, and each
//! file's size and SHA-256. Pre-releases leave the beta's
//! `releases/latest` link alone. Old releases are kept.
//!
//! The launcher knows its own build from `M2_BUILD_SHA` and the releases'
//! repository from `M2_RELEASE_REPO`, both set by the workflow as it
//! builds. A build made without them (a local `cargo build`) has no
//! repository to look in and never checks.
//!
//! # Checking
//!
//! On open, one unauthenticated request to the public API lists the
//! repository's releases (`GET /repos/<repo>/releases`); the newest
//! `build-` one that is not a draft is the latest. Its manifest is then
//! downloaded. The requests carry only a generic User-Agent; a short
//! timeout keeps an offline launcher as it was. They go through the
//! `curl.exe` that ships with Windows 10 and 11.
//!
//! # Swapping
//!
//! The new files are downloaded into `update\staged\` beside the launcher
//! and each is checked against the manifest. Then each current file is
//! moved to `update\previous\` and the staged one moved into its place.
//! Windows will not overwrite a running program, but it does let one be
//! renamed, so the running launcher is moved aside like the rest and the
//! new one runs from the next start (the launcher offers to restart).
//! Only the names in [`FILES`] are ever moved: saves, settings and
//! anything else in the folder are not touched. [`rollback`] swaps
//! `update\previous\` back in the same way.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The files a build holds: our two programs, nothing else.
pub const FILES: [&str; 2] = ["m2-alpha.exe", "m2-launcher.exe"];
/// The manifest each release carries.
pub const MANIFEST: &str = "manifest.toml";
/// The release tags' prefix.
pub const TAG_PREFIX: &str = "build-";
/// The folder beside the launcher that holds the staged and the previous
/// build.
pub const UPDATE_DIR: &str = "update";

/// This build's commit, if the release workflow built it.
pub const BUILD_SHA: Option<&str> = option_env!("M2_BUILD_SHA");
/// The repository the releases are in (`owner/name`), if the release
/// workflow built it.
pub const RELEASE_REPO: Option<&str> = option_env!("M2_RELEASE_REPO");

/// One file of a build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

/// A build's manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub tag: String,
    /// The commit it was built from.
    pub sha: String,
    /// When, as the workflow wrote it (UTC).
    #[serde(default)]
    pub built: String,
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| e.message().to_owned())?;
        for f in &m.files {
            allowed(&f.name)?;
            if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("{}: not a SHA-256", f.name));
            }
        }
        if m.files.is_empty() {
            return Err("no files".to_owned());
        }
        Ok(m)
    }

    pub fn to_text(&self) -> String {
        toml::to_string_pretty(self).expect("a manifest always serialises")
    }

    /// The short commit, as the page shows it.
    pub fn short(&self) -> &str {
        short(&self.sha)
    }
}

pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// A name the updater may write: one of [`FILES`], a bare file name.
fn allowed(name: &str) -> Result<(), String> {
    if FILES.contains(&name) {
        Ok(())
    } else {
        Err(format!("{name:?} is not a file a build holds"))
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The manifest for the files of [`FILES`] found in `dir`.
pub fn manifest_for(dir: &Path, tag: &str, sha: &str, built: &str) -> Result<Manifest, String> {
    let mut files = Vec::new();
    for name in FILES {
        let path = dir.join(name);
        if let Ok(bytes) = std::fs::read(&path) {
            files.push(FileEntry {
                name: name.to_owned(),
                size: bytes.len() as u64,
                sha256: sha256_hex(&bytes),
            });
        }
    }
    if files.is_empty() {
        return Err(format!("none of {FILES:?} in {}", dir.display()));
    }
    Ok(Manifest {
        tag: tag.to_owned(),
        sha: sha.to_owned(),
        built: built.to_owned(),
        files,
    })
}

/// `SHA256SUMS`: `<hex>  <name>` per file, as `sha256sum` writes it.
pub fn sums_text(m: &Manifest) -> String {
    m.files
        .iter()
        .map(|f| format!("{}  {}\n", f.sha256, f.name))
        .collect()
}

/// A release as the API lists it (only what is used).
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// The newest published `build-` release in the API's list.
pub fn latest(json: &str) -> Result<Option<Release>, String> {
    let mut list: Vec<Release> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    list.retain(|r| !r.draft && r.tag_name.starts_with(TAG_PREFIX));
    // RFC 3339 times in UTC sort as text.
    list.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    Ok(list.into_iter().next())
}

impl Release {
    pub fn asset_url(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.as_str())
    }
}

/// How this build stands against the latest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    UpToDate,
    /// Another build is out.
    Behind,
    /// This build's commit is unknown: an update can be offered, never
    /// done on its own.
    Unknown,
}

pub fn standing(current: Option<&str>, latest: &Manifest) -> Standing {
    match current {
        None => Standing::Unknown,
        Some(c) if c.eq_ignore_ascii_case(&latest.sha) => Standing::UpToDate,
        Some(_) => Standing::Behind,
    }
}

fn staged_dir(dir: &Path) -> PathBuf {
    dir.join(UPDATE_DIR).join("staged")
}

fn previous_dir(dir: &Path) -> PathBuf {
    dir.join(UPDATE_DIR).join("previous")
}

/// Downloads `m`'s files into `update\staged\` through `fetch` and checks
/// each against it. Nothing outside `update\` changes.
pub fn stage(
    dir: &Path,
    m: &Manifest,
    fetch: &mut dyn FnMut(&FileEntry) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let staged = staged_dir(dir);
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::create_dir_all(&staged).map_err(|e| e.to_string())?;
    for f in &m.files {
        allowed(&f.name)?;
        let bytes = fetch(f)?;
        if bytes.len() as u64 != f.size || sha256_hex(&bytes) != f.sha256.to_ascii_lowercase() {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(format!(
                "{}: the download does not match its checksum",
                f.name
            ));
        }
        std::fs::write(staged.join(&f.name), &bytes).map_err(|e| e.to_string())?;
    }
    std::fs::write(staged.join(MANIFEST), m.to_text()).map_err(|e| e.to_string())?;
    Ok(())
}

/// A staged build waiting to be swapped in, if there is one.
pub fn staged(dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(staged_dir(dir).join(MANIFEST)).ok()?;
    Manifest::parse(&text).ok()
}

/// The build kept for rolling back, if there is one.
pub fn previous(dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(previous_dir(dir).join(MANIFEST)).ok()?;
    Manifest::parse(&text).ok()
}

/// Moves `from`'s files (named in `m`) into `dir`, the ones there now into
/// `aside`, and records them there as `current` (the build they were:
/// its commit, if known). Undoes what it did if a move fails. The names
/// moved come back.
fn swap_in(
    dir: &Path,
    from: &Path,
    aside: &Path,
    m: &Manifest,
    current: &Manifest,
) -> Result<Vec<String>, String> {
    // Check every file first, so a bad one changes nothing.
    for f in &m.files {
        allowed(&f.name)?;
        let bytes = std::fs::read(from.join(&f.name)).map_err(|e| format!("{}: {e}", f.name))?;
        if sha256_hex(&bytes) != f.sha256.to_ascii_lowercase() {
            return Err(format!("{}: does not match its checksum", f.name));
        }
    }
    let hold = dir.join(UPDATE_DIR).join("swapping");
    let _ = std::fs::remove_dir_all(&hold);
    std::fs::create_dir_all(&hold).map_err(|e| e.to_string())?;
    // (name, moved the old one aside, moved the new one in)
    let mut done: Vec<(String, bool, bool)> = Vec::new();
    let mut result = Ok(());
    for f in &m.files {
        let live = dir.join(&f.name);
        let had = live.is_file();
        if had && let Err(e) = std::fs::rename(&live, hold.join(&f.name)) {
            result = Err(format!("{}: {e}", f.name));
            break;
        }
        done.push((f.name.clone(), had, false));
        if let Err(e) = std::fs::rename(from.join(&f.name), &live) {
            result = Err(format!("{}: {e}", f.name));
            break;
        }
        if let Some(d) = done.last_mut() {
            d.2 = true;
        }
    }
    if let Err(e) = result {
        for (name, had, moved_in) in done.into_iter().rev() {
            if moved_in {
                let _ = std::fs::rename(dir.join(&name), from.join(&name));
            }
            if had {
                let _ = std::fs::rename(hold.join(&name), dir.join(&name));
            }
        }
        let _ = std::fs::remove_dir_all(&hold);
        return Err(e);
    }
    // What was there is now the build to roll back to.
    let _ = std::fs::remove_dir_all(aside);
    std::fs::rename(&hold, aside).map_err(|e| e.to_string())?;
    let mut kept = current.clone();
    kept.files.retain(|f| aside.join(&f.name).is_file());
    std::fs::write(aside.join(MANIFEST), kept.to_text()).map_err(|e| e.to_string())?;
    Ok(done.into_iter().map(|(n, ..)| n).collect())
}

/// The running build's manifest, from the files in `dir` (its commit
/// `current`, or "unknown").
fn current_manifest(dir: &Path, current: Option<&str>) -> Manifest {
    let sha = current.unwrap_or("unknown");
    manifest_for(dir, &format!("{TAG_PREFIX}{}", short(sha)), sha, "").unwrap_or(Manifest {
        tag: String::new(),
        sha: sha.to_owned(),
        built: String::new(),
        files: Vec::new(),
    })
}

/// Swaps the staged build in; the build there now is kept for
/// [`rollback`]. The names replaced come back.
pub fn apply(dir: &Path, current: Option<&str>) -> Result<Vec<String>, String> {
    let m = staged(dir).ok_or("nothing is staged")?;
    let now = current_manifest(dir, current);
    let staged = staged_dir(dir);
    let out = swap_in(dir, &staged, &previous_dir(dir), &m, &now)?;
    let _ = std::fs::remove_dir_all(&staged);
    Ok(out)
}

/// Swaps the previous build back in; the one there now becomes the
/// previous (so a rollback can itself be undone).
pub fn rollback(dir: &Path, current: Option<&str>) -> Result<(Manifest, Vec<String>), String> {
    let m = previous(dir).ok_or("no previous build is kept")?;
    let now = current_manifest(dir, current);
    let back = dir.join(UPDATE_DIR).join("rolling-back");
    let _ = std::fs::remove_dir_all(&back);
    std::fs::rename(previous_dir(dir), &back).map_err(|e| e.to_string())?;
    match swap_in(dir, &back, &previous_dir(dir), &m, &now) {
        Ok(names) => {
            let _ = std::fs::remove_dir_all(&back);
            Ok((m, names))
        }
        Err(e) => {
            let _ = std::fs::rename(&back, previous_dir(dir));
            Err(e)
        }
    }
}

// ---- the network: `curl.exe`, never in tests ----

const USER_AGENT: &str = "m2-launcher";

fn curl(url: &str, seconds: u32) -> Result<Vec<u8>, String> {
    let mut cmd = std::process::Command::new("curl.exe");
    cmd.args([
        "-fsSL",
        "--max-time",
        &seconds.to_string(),
        "-A",
        USER_AGENT,
    ])
    .args(["-H", "Accept: application/vnd.github+json"])
    .arg(url)
    .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // No console window flashes up from the windowed launcher.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| format!("curl: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "{url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The latest build out there, with where its files are.
#[derive(Clone, Debug)]
pub struct Found {
    pub manifest: Manifest,
    pub release: Release,
}

/// Looks for the latest build (needs [`RELEASE_REPO`]).
pub fn check() -> Result<Option<Found>, String> {
    let Some(repo) = RELEASE_REPO else {
        return Ok(None);
    };
    let list = curl(
        &format!("https://api.github.com/repos/{repo}/releases?per_page=30"),
        10,
    )?;
    let Some(release) = latest(&String::from_utf8_lossy(&list))? else {
        return Ok(None);
    };
    let url = release
        .asset_url(MANIFEST)
        .ok_or_else(|| format!("{}: no {MANIFEST}", release.tag_name))?;
    let text = curl(url, 10)?;
    let manifest = Manifest::parse(&String::from_utf8_lossy(&text))?;
    Ok(Some(Found { manifest, release }))
}

/// Downloads and swaps in `found` beside the launcher in `dir`.
pub fn update(dir: &Path, found: &Found) -> Result<Vec<String>, String> {
    stage(dir, &found.manifest, &mut |f| {
        let url = found
            .release
            .asset_url(&f.name)
            .ok_or_else(|| format!("{}: not in the release", f.name))?;
        curl(url, 600)
    })?;
    apply(dir, BUILD_SHA)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("m2-launcher-upd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn manifest(sha: &str, files: &[(&str, &[u8])]) -> Manifest {
        Manifest {
            tag: format!("build-20261126-{}", short(sha)),
            sha: sha.to_owned(),
            built: "2026-11-26T02:00:00Z".to_owned(),
            files: files
                .iter()
                .map(|(n, b)| FileEntry {
                    name: (*n).to_owned(),
                    size: b.len() as u64,
                    sha256: sha256_hex(b),
                })
                .collect(),
        }
    }

    /// A releases list as the API returns one (only the fields read, and
    /// a few it ignores).
    const RELEASES: &str = r#"[
      {"tag_name":"beta-20261101","draft":false,"prerelease":false,"published_at":"2026-11-01T10:00:00Z","assets":[]},
      {"tag_name":"build-20261125-1111111","draft":false,"prerelease":true,"published_at":"2026-11-25T10:00:00Z","assets":[]},
      {"tag_name":"build-20261126-2222222","draft":false,"prerelease":true,"published_at":"2026-11-26T09:00:00Z",
       "assets":[{"name":"manifest.toml","browser_download_url":"https://example.invalid/m","size":1},
                 {"name":"m2-alpha.exe","browser_download_url":"https://example.invalid/a"}]},
      {"tag_name":"build-20261127-3333333","draft":true,"published_at":null,"assets":[]}
    ]"#;

    #[test]
    fn the_latest_is_the_newest_published_build() {
        let r = latest(RELEASES).unwrap().unwrap();
        assert_eq!(r.tag_name, "build-20261126-2222222");
        assert_eq!(
            r.asset_url("manifest.toml"),
            Some("https://example.invalid/m")
        );
        assert_eq!(r.asset_url("m2-launcher.exe"), None);
        assert!(latest("[]").unwrap().is_none());
        assert!(latest("{not json").is_err());
    }

    #[test]
    fn a_manifest_round_trips_and_names_only_our_programs() {
        let m = manifest(
            "2222222abcdef",
            &[("m2-alpha.exe", b"alpha"), ("m2-launcher.exe", b"l")],
        );
        assert_eq!(Manifest::parse(&m.to_text()).unwrap(), m);
        assert_eq!(m.short(), "2222222");
        assert!(sums_text(&m).starts_with(&format!("{}  m2-alpha.exe\n", sha256_hex(b"alpha"))));
        // Anything else, a path, or a bad sum is refused.
        for bad in [
            "m2-alpha-save1.sav",
            "m2-launcher.toml",
            "..\\m2-alpha.exe",
            "sub/m2-alpha.exe",
        ] {
            let mut m2 = m.clone();
            m2.files[0].name = bad.to_owned();
            assert!(Manifest::parse(&m2.to_text()).is_err(), "{bad}");
        }
        let mut m3 = m.clone();
        m3.files[0].sha256 = "xyz".to_owned();
        assert!(Manifest::parse(&m3.to_text()).is_err());
        assert!(Manifest::parse("tag = 'x'\nsha = 'y'\n").is_err());
    }

    #[test]
    fn standing_compares_commits() {
        let m = manifest("ABCDEF1234", &[("m2-alpha.exe", b"a")]);
        assert_eq!(standing(Some("abcdef1234"), &m), Standing::UpToDate);
        assert_eq!(standing(Some("0000000"), &m), Standing::Behind);
        assert_eq!(standing(None, &m), Standing::Unknown);
    }

    #[test]
    fn hashes_are_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn staging_checks_and_swapping_keeps_the_old_build_and_the_saves() {
        let d = dir("swap");
        std::fs::write(d.join("m2-alpha.exe"), b"old alpha").unwrap();
        std::fs::write(d.join("m2-launcher.exe"), b"old launcher").unwrap();
        std::fs::write(d.join("m2-alpha-save1.sav"), b"save").unwrap();
        std::fs::write(d.join("m2-launcher.toml"), b"settings").unwrap();
        let new = manifest(
            "2222222",
            &[
                ("m2-alpha.exe", b"new alpha"),
                ("m2-launcher.exe", b"new launcher"),
            ],
        );
        // A download that does not match stages nothing.
        let bad = stage(&d, &new, &mut |_| Ok(b"tampered".to_vec()));
        assert!(bad.unwrap_err().contains("checksum"));
        assert!(staged(&d).is_none());
        assert_eq!(std::fs::read(d.join("m2-alpha.exe")).unwrap(), b"old alpha");
        // A good one stages, then swaps in.
        let files: Vec<(String, Vec<u8>)> = vec![
            ("m2-alpha.exe".into(), b"new alpha".to_vec()),
            ("m2-launcher.exe".into(), b"new launcher".to_vec()),
        ];
        stage(&d, &new, &mut |f| {
            Ok(files.iter().find(|(n, _)| *n == f.name).unwrap().1.clone())
        })
        .unwrap();
        assert_eq!(staged(&d).unwrap(), new);
        let names = apply(&d, Some("1111111")).unwrap();
        assert_eq!(names, ["m2-alpha.exe", "m2-launcher.exe"]);
        assert_eq!(std::fs::read(d.join("m2-alpha.exe")).unwrap(), b"new alpha");
        assert_eq!(
            std::fs::read(d.join("m2-launcher.exe")).unwrap(),
            b"new launcher"
        );
        assert!(staged(&d).is_none());
        let prev = previous(&d).unwrap();
        assert_eq!(prev.sha, "1111111");
        assert_eq!(prev.files.len(), 2);
        // Saves and settings are untouched.
        assert_eq!(
            std::fs::read(d.join("m2-alpha-save1.sav")).unwrap(),
            b"save"
        );
        assert_eq!(
            std::fs::read(d.join("m2-launcher.toml")).unwrap(),
            b"settings"
        );
        // Rolling back brings the old build back and keeps the new one.
        let (back, _) = rollback(&d, Some("2222222")).unwrap();
        assert_eq!(back.sha, "1111111");
        assert_eq!(std::fs::read(d.join("m2-alpha.exe")).unwrap(), b"old alpha");
        assert_eq!(
            std::fs::read(d.join("m2-launcher.exe")).unwrap(),
            b"old launcher"
        );
        assert_eq!(previous(&d).unwrap().sha, "2222222");
        // And forward again.
        rollback(&d, Some("1111111")).unwrap();
        assert_eq!(std::fs::read(d.join("m2-alpha.exe")).unwrap(), b"new alpha");
        assert_eq!(
            std::fs::read(d.join("m2-alpha-save1.sav")).unwrap(),
            b"save"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_staged_file_changed_since_is_not_swapped_in() {
        let d = dir("tamper");
        std::fs::write(d.join("m2-alpha.exe"), b"old").unwrap();
        let new = manifest("2222222", &[("m2-alpha.exe", b"new")]);
        stage(&d, &new, &mut |_| Ok(b"new".to_vec())).unwrap();
        std::fs::write(
            d.join("update").join("staged").join("m2-alpha.exe"),
            b"evil",
        )
        .unwrap();
        assert!(apply(&d, None).is_err());
        assert_eq!(std::fs::read(d.join("m2-alpha.exe")).unwrap(), b"old");
        assert!(rollback(&d, None).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_manifest_is_made_from_the_files_there() {
        let d = dir("make");
        assert!(manifest_for(&d, "t", "s", "").is_err());
        std::fs::write(d.join("m2-alpha.exe"), b"alpha").unwrap();
        std::fs::write(d.join("mercs2.ico"), b"not ours").unwrap();
        let m = manifest_for(&d, "build-x", "abc", "now").unwrap();
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files[0].name, "m2-alpha.exe");
        assert_eq!(m.files[0].size, 5);
        let _ = std::fs::remove_dir_all(&d);
    }
}
