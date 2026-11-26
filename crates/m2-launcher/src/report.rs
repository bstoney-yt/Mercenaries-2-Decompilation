//! Bug reports: the game's `last-run.log`, the build's details and the
//! launcher's settings, redacted and zipped into a local folder
//! (`%LOCALAPPDATA%\m2-alpha\Reports`) for the player to attach by hand,
//! and a short text-only issue template for the clipboard. Nothing is
//! uploaded or sent anywhere. Saves and the game's files are never taken:
//! only the three files named in [`bundle`].
//!
//! Redaction ([`Redactor`]): the player's home folder and any
//! `\Users\<name>` or `/home/<name>` path part, the machine's name, IPv4
//! addresses, and token-shaped values (GitHub tokens, `Bearer` values, and
//! values after `token`, `password`, `secret` or `api_key`).
//!
//! Everything here is the launcher's own (**invented**).

use std::path::{Path, PathBuf};

use crate::update;

/// The game's log, beside its settings (`m2-alpha`'s `errlog`).
pub const LOG_FILE: &str = "last-run.log";
/// Only the end of a long log is taken, bytes.
pub const LOG_MAX: usize = 2 * 1024 * 1024;

/// The build and the machine, as far as they are known.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub version: String,
    pub commit: String,
    pub built: String,
    pub os: String,
    pub gpu: Option<String>,
}

impl Info {
    /// This launcher's build, and the GPU the game's log named.
    pub fn gather(log: Option<&str>) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commit: update::BUILD_SHA
                .unwrap_or("unknown (a local build)")
                .to_owned(),
            built: option_env!("M2_BUILD_DATE").unwrap_or("unknown").to_owned(),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            gpu: log.and_then(gpu_from_log),
        }
    }

    pub fn text(&self) -> String {
        format!(
            "m2-launcher {}\ncommit: {}\nbuilt: {}\nos: {}\ngpu: {}\n",
            self.version,
            self.commit,
            self.built,
            self.os,
            self.gpu.as_deref().unwrap_or("unknown")
        )
    }
}

/// The adapter's name from the game's `adapter: AdapterSummary { name: "..."` line.
pub fn gpu_from_log(log: &str) -> Option<String> {
    let line = log.lines().find(|l| l.starts_with("adapter: "))?;
    let i = line.find("name: \"")? + 7;
    let rest = &line[i..];
    Some(rest[..rest.find('"')?].to_owned())
}

/// What to take out of a report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Redactor {
    /// The home folder, as the environment gives it.
    pub home: Option<String>,
    /// The machine's name.
    pub machine: Option<String>,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Replaces every whole-word, case-insensitive `needle` in `text`.
fn replace_word(text: &str, needle: &str, with: &str) -> String {
    if needle.len() < 2 {
        return text.to_owned();
    }
    let lower = text.to_ascii_lowercase();
    let n = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut from = 0;
    while let Some(i) = lower[from..].find(&n).map(|i| i + from) {
        let end = i + n.len();
        let before = text[..i].chars().next_back().is_none_or(|c| !is_word(c));
        let after = text[end..].chars().next().is_none_or(|c| !is_word(c));
        if before && after {
            out.push_str(&text[last..i]);
            out.push_str(with);
            last = end;
        }
        from = end;
    }
    out.push_str(&text[last..]);
    out
}

/// Replaces every case-insensitive `needle` in `text`, with either slash.
fn replace_path(text: &str, needle: &str, with: &str) -> String {
    if needle.len() < 3 {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    // As written with backslashes, escaped (TOML, JSON: `\\`), and with
    // forward slashes.
    let back = needle.replace('/', "\\");
    for n in [back.replace('\\', "\\\\"), back, needle.replace('\\', "/")] {
        let lower = out.to_ascii_lowercase();
        let nl = n.to_ascii_lowercase();
        let mut s = String::with_capacity(out.len());
        let mut last = 0;
        let mut from = 0;
        while let Some(i) = lower[from..].find(&nl).map(|i| i + from) {
            s.push_str(&out[last..i]);
            s.push_str(with);
            last = i + nl.len();
            from = last;
        }
        s.push_str(&out[last..]);
        out = s;
    }
    out
}

/// `\Users\<name>` and `/home/<name>` (any name) to `<user>`.
fn redact_user_dirs(text: &str) -> String {
    let mut out = text.to_owned();
    for marker in ["\\\\users\\\\", "\\users\\", "/users/", "/home/"] {
        let mut s = String::with_capacity(out.len());
        let lower = out.to_ascii_lowercase();
        let mut last = 0;
        let mut from = 0;
        while let Some(i) = lower[from..].find(marker).map(|i| i + from) {
            let start = i + marker.len();
            let end = out[start..]
                .find(|c: char| c == '\\' || c == '/' || c == '"' || c == '\'' || c.is_whitespace())
                .map_or(out.len(), |e| start + e);
            s.push_str(&out[last..start]);
            if end > start && &out[start..end] != "<user>" {
                s.push_str("<user>");
            } else {
                s.push_str(&out[start..end]);
            }
            last = end;
            from = end.max(start);
        }
        s.push_str(&out[last..]);
        out = s;
    }
    out
}

/// Dotted IPv4 addresses to `<ip>`.
fn redact_ips(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i].is_ascii_digit()
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.'))
        {
            // Four groups of 1-3 digits, each 0-255, separated by dots.
            let mut j = i;
            let mut ok = true;
            for part in 0..4 {
                let s = j;
                while j < b.len() && b[j].is_ascii_digit() && j - s < 4 {
                    j += 1;
                }
                let len = j - s;
                if len == 0 || len > 3 || text[s..j].parse::<u32>().map_or(true, |v| v > 255) {
                    ok = false;
                    break;
                }
                if part < 3 {
                    if j < b.len() && b[j] == b'.' {
                        j += 1;
                    } else {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && (j == b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'.')) {
                out.push_str(&text[last..i]);
                out.push_str("<ip>");
                last = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&text[last..]);
    out
}

/// Token-shaped values to `<token>`.
fn redact_tokens(text: &str) -> String {
    let token_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.';
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let mut l = line.to_owned();
        // GitHub tokens by prefix.
        for prefix in ["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
            while let Some(i) = l.find(prefix) {
                let end = l[i..]
                    .find(|c: char| !token_char(c))
                    .map_or(l.len(), |e| i + e);
                l.replace_range(i..end, "<token>");
            }
        }
        // Values after a key that names a secret.
        let lower = l.to_ascii_lowercase();
        for key in [
            "bearer ",
            "token",
            "password",
            "secret",
            "api_key",
            "apikey",
            "authorization",
        ] {
            if let Some(k) = lower.find(key) {
                let mut v = k + key.len();
                let bytes = l.as_bytes();
                while v < bytes.len() && matches!(bytes[v], b' ' | b'=' | b':' | b'"' | b'\'') {
                    v += 1;
                }
                let end = l[v..]
                    .find(|c: char| !token_char(c))
                    .map_or(l.len(), |e| v + e);
                if end > v && &l[v..end] != "<token>" && (key == "bearer " || end - v >= 6) {
                    l.replace_range(v..end, "<token>");
                }
                break;
            }
        }
        out.push_str(&l);
    }
    out
}

impl Redactor {
    /// From the environment `env` reads (`USERPROFILE` or `HOME`,
    /// `COMPUTERNAME` or `HOSTNAME`).
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            home: env("USERPROFILE")
                .or_else(|| env("HOME"))
                .filter(|h| h.len() > 3),
            machine: env("COMPUTERNAME")
                .or_else(|| env("HOSTNAME"))
                .filter(|m| m.len() >= 3),
        }
    }

    pub fn redact(&self, text: &str) -> String {
        let mut t = text.to_owned();
        if let Some(h) = &self.home {
            t = replace_path(&t, h, "<home>");
        }
        t = redact_user_dirs(&t);
        if let Some(m) = &self.machine {
            t = replace_word(&t, m, "<machine>");
        }
        t = redact_ips(&t);
        redact_tokens(&t)
    }
}

/// The report's files, redacted: `build-info.txt`, the end of the game's
/// log (if there is one) and the launcher's settings. Nothing else.
pub fn bundle(
    log: Option<&str>,
    settings_toml: &str,
    info: &Info,
    r: &Redactor,
) -> Vec<(String, Vec<u8>)> {
    let mut files = vec![(
        "build-info.txt".to_owned(),
        r.redact(&info.text()).into_bytes(),
    )];
    if let Some(log) = log {
        let start = log.len().saturating_sub(LOG_MAX);
        let start = (start..log.len())
            .find(|&i| log.is_char_boundary(i))
            .unwrap_or(log.len());
        files.push((LOG_FILE.to_owned(), r.redact(&log[start..]).into_bytes()));
    }
    files.push((
        "m2-launcher.toml".to_owned(),
        r.redact(settings_toml).into_bytes(),
    ));
    files
}

/// A line per file, for the page's preview.
pub fn preview(files: &[(String, Vec<u8>)]) -> Vec<String> {
    files
        .iter()
        .map(|(n, b)| format!("{n} ({} bytes, redacted)", b.len()))
        .collect()
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zip of `files`, deflated (the format's method 8).
pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let packed = miniz_oxide::deflate::compress_to_vec(data, 6);
        let crc = crc32(data);
        let offset = out.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend(sig.to_le_bytes());
            if central {
                h.extend(20u16.to_le_bytes()); // made by
            }
            h.extend(20u16.to_le_bytes()); // needed
            h.extend(0u16.to_le_bytes()); // flags
            h.extend(8u16.to_le_bytes()); // deflate
            h.extend(0u16.to_le_bytes()); // time
            h.extend(0x21u16.to_le_bytes()); // date: 1980-01-01
            h.extend(crc.to_le_bytes());
            h.extend((packed.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra
            if central {
                h.extend(0u16.to_le_bytes()); // comment
                h.extend(0u16.to_le_bytes()); // disk
                h.extend(0u16.to_le_bytes()); // internal attributes
                h.extend(0u32.to_le_bytes()); // external attributes
                h.extend(offset.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend(&packed);
        central.extend(header(0x0201_4b50, true));
    }
    let at = out.len() as u32;
    let size = central.len() as u32;
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]); // disks
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// Where reports go: `M2_REPORTS` if set, else `Reports` under the user's
/// data folder (`%LOCALAPPDATA%\m2-alpha`, or `$XDG_DATA_HOME/m2-alpha` /
/// `~/.local/share/m2-alpha`).
pub fn reports_dir(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(d) = env("M2_REPORTS").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let base = env("LOCALAPPDATA")
        .or_else(|| env("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|h| Path::new(&h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("m2-alpha").join("Reports")
}

/// Writes the zip into `dir` as `bug-report-<unix>.zip`.
pub fn write(dir: &Path, files: &[(String, Vec<u8>)], unix: u64) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("bug-report-{unix}.zip"));
    std::fs::write(&path, zip(files)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// The short text for a GitHub issue: the build's details and prompts,
/// no log and no paths.
pub fn issue_text(info: &Info) -> String {
    format!(
        "### What happened\n\n<!-- What did you do, what did you expect, what happened instead? -->\n\n\
### Steps to reproduce\n\n1. \n\n\
### Build\n\n- launcher {}\n- commit {}\n- built {}\n- os {}\n- gpu {}\n\n\
<!-- A bug-report zip from the launcher's Options page can be attached here if you want: \
it holds the game's log, redacted. -->\n",
        info.version,
        info.commit,
        info.built,
        info.os,
        info.gpu.as_deref().unwrap_or("unknown")
    )
}

/// Opens `dir` in the system's file browser.
pub fn open_folder(dir: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(dir).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Redactor {
        Redactor {
            home: Some("C:\\Users\\alice".into()),
            machine: Some("ALICE-PC".into()),
        }
    }

    const LOG: &str = "adapter: AdapterSummary { name: \"Test GPU 9000\", backend: \"Vulkan\" }\n\
settings: C:\\Users\\alice\\AppData\\Local\\m2\\m2-alpha.toml\n\
saves beside c:/users/alice/Documents and C:\\Users\\bob\\x.sav\n\
host alice-pc joined from 192.168.1.20:4000, not 1.2.3 or v10.0.26300.1\n\
coop: invite link token=abcdef123456 and ghp_ABCDEFGHIJKLMNOPQRSTUV0123\n\
Authorization: Bearer xyz.abc-123\n\
--dev keeps the word dev in prose\n";

    #[test]
    fn redaction_takes_out_people_machines_addresses_and_tokens() {
        let t = r().redact(LOG);
        for gone in [
            "alice\\",
            "Users\\alice",
            "users/alice",
            "bob",
            "ALICE-PC",
            "alice-pc",
            "192.168",
            "abcdef123456",
            "ghp_",
            "xyz.abc",
        ] {
            assert!(
                !t.to_ascii_lowercase().contains(&gone.to_ascii_lowercase()),
                "{gone} left in:\n{t}"
            );
        }
        assert!(
            t.contains("<home>\\AppData\\Local\\m2\\m2-alpha.toml"),
            "{t}"
        );
        assert!(
            t.contains("c:/users/<user>/Documents") || t.contains("<home>/Documents"),
            "{t}"
        );
        assert!(t.contains("C:\\Users\\<user>\\x.sav"), "{t}");
        assert!(t.contains("host <machine> joined from <ip>:4000"), "{t}");
        // Not an address: three parts, or part of a version.
        assert!(t.contains("not 1.2.3 or v10.0.26300.1"), "{t}");
        assert!(
            t.contains("token=<token>") && t.contains("Bearer <token>"),
            "{t}"
        );
        // Ordinary words stay.
        assert!(t.contains("--dev keeps the word dev"), "{t}");
        assert!(t.contains("Test GPU 9000"));
    }

    #[test]
    fn the_gpu_comes_from_the_log() {
        assert_eq!(gpu_from_log(LOG).as_deref(), Some("Test GPU 9000"));
        assert_eq!(gpu_from_log("nothing"), None);
    }

    #[test]
    fn the_environment_names_what_to_redact() {
        let env = |k: &str| match k {
            "USERPROFILE" => Some("C:\\Users\\carol".to_owned()),
            "COMPUTERNAME" => Some("BOX1".to_owned()),
            _ => None,
        };
        let red = Redactor::from_env(env);
        assert_eq!(red.home.as_deref(), Some("C:\\Users\\carol"));
        assert_eq!(red.machine.as_deref(), Some("BOX1"));
        assert_eq!(
            red.redact("on BOX1 at C:\\Users\\carol\\x"),
            "on <machine> at <home>\\x"
        );
    }

    /// Reads a zip's entries back (stored names, inflated data), checking
    /// each CRC.
    fn unzip(z: &[u8]) -> Vec<(String, Vec<u8>)> {
        let u16_at = |i: usize| u16::from_le_bytes([z[i], z[i + 1]]) as usize;
        let u32_at = |i: usize| u32::from_le_bytes([z[i], z[i + 1], z[i + 2], z[i + 3]]) as usize;
        let mut out = Vec::new();
        let mut i = 0;
        while u32_at(i) == 0x0403_4b50 {
            let crc = u32_at(i + 14) as u32;
            let packed = u32_at(i + 18);
            let name_len = u16_at(i + 26);
            let extra = u16_at(i + 28);
            let name = String::from_utf8(z[i + 30..i + 30 + name_len].to_vec()).unwrap();
            let start = i + 30 + name_len + extra;
            let data = miniz_oxide::inflate::decompress_to_vec(&z[start..start + packed]).unwrap();
            assert_eq!(crc32(&data), crc);
            out.push((name, data));
            i = start + packed;
        }
        // The end record counts them.
        let end = z.len() - 22;
        assert_eq!(u32_at(end), 0x0605_4b50);
        assert_eq!(u16_at(end + 10), out.len());
        out
    }

    #[test]
    fn the_zip_holds_exactly_the_redacted_files() {
        let info = Info {
            version: "0.1.0".into(),
            commit: "abc".into(),
            built: "2026-11-26".into(),
            os: "windows x86_64".into(),
            gpu: gpu_from_log(LOG),
        };
        let files = bundle(
            Some(LOG),
            "install = \"C:\\\\Users\\\\alice\\\\Games\"\n",
            &info,
            &r(),
        );
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["build-info.txt", "last-run.log", "m2-launcher.toml"]
        );
        let back = unzip(&zip(&files));
        assert_eq!(back, files);
        let all: String = back
            .iter()
            .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
            .collect();
        assert!(!all.to_ascii_lowercase().contains("alice"), "{all}");
        assert!(
            !all.contains(".sav\n") || all.contains("<user>"),
            "no save file is taken, only a path"
        );
        // Without a log, two files.
        assert_eq!(bundle(None, "", &info, &r()).len(), 2);
        assert_eq!(
            preview(&files)[1],
            format!("last-run.log ({} bytes, redacted)", files[1].1.len())
        );
    }

    #[test]
    fn a_long_log_keeps_its_end() {
        let log = "x".repeat(LOG_MAX + 10) + "\nthe end\n";
        let info = Info::gather(None);
        let files = bundle(Some(&log), "", &info, &Redactor::default());
        let kept = String::from_utf8(files[1].1.clone()).unwrap();
        assert_eq!(kept.len(), LOG_MAX);
        assert!(kept.ends_with("the end\n"));
    }

    #[test]
    fn issue_text_has_no_log_or_paths() {
        let info = Info {
            version: "0.1.0".into(),
            commit: "abc1234".into(),
            built: "unknown".into(),
            os: "windows x86_64".into(),
            gpu: Some("Test GPU".into()),
        };
        let t = issue_text(&info);
        assert!(t.contains("- commit abc1234") && t.contains("- gpu Test GPU"));
        assert!(!t.contains(":\\") && !t.contains("adapter:"));
    }

    #[test]
    fn reports_go_to_the_users_data_folder() {
        let env = |k: &str| (k == "LOCALAPPDATA").then(|| "C:/L".to_owned());
        assert_eq!(reports_dir(env), Path::new("C:/L/m2-alpha/Reports"));
        let d = std::env::temp_dir().join(format!("m2-report-{}", std::process::id()));
        let p = write(&d, &[("a.txt".into(), b"hi".to_vec())], 42).unwrap();
        assert_eq!(p.file_name().unwrap(), "bug-report-42.zip");
        assert_eq!(
            unzip(&std::fs::read(&p).unwrap()),
            [("a.txt".to_owned(), b"hi".to_vec())]
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
