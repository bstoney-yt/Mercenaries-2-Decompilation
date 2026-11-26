//! Bink movies (`data\Movies\*.bik`) as `m2-alpha` and the launcher
//! play them through a separate `ffmpeg` process: the header's size and
//! rate, where `ffmpeg` is, and how the sound tracks are put together.
//! Moved here from `m2-alpha`'s `movie.rs` so the launcher's gallery
//! shares them; behaviour unchanged.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A movie's size and rate, from its Bink header.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BinkInfo {
    pub frames: u32,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl BinkInfo {
    /// Reads the header of a Bink 1 file (`BIK` and a revision letter;
    /// then size, frames, largest frame, frames again, width, height and
    /// the frame rate as a fraction, each a little-endian u32).
    pub fn parse(head: &[u8]) -> Option<Self> {
        if head.len() < 36 || &head[..3] != b"BIK" {
            return None;
        }
        let u = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap());
        let (num, den) = (u(28), u(32));
        if den == 0 || num == 0 || u(20) == 0 || u(24) == 0 {
            return None;
        }
        Some(Self {
            frames: u(8),
            width: u(20),
            height: u(24),
            fps: f64::from(num) / f64::from(den),
        })
    }

    pub fn seconds(&self) -> f64 {
        f64::from(self.frames) / self.fps
    }
}

/// Where `ffmpeg` is, if anywhere.
pub fn ffmpeg() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("M2_FFMPEG").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(exe)));
    if let Some(b) = beside.filter(|b| b.is_file()) {
        return Some(b);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(exe))
            .find(|p| p.is_file())
    })
}

/// The Bink track ids of a movie's sound tracks, as `ffmpeg` lists them
/// (`Stream #0:4[0x7d1]: Audio: ...`).
pub fn audio_tracks(ffmpeg: &Path, file: &Path) -> Vec<u32> {
    let Ok(out) = quiet(Command::new(ffmpeg).args(["-hide_banner", "-i"]).arg(file))
        .stderr(Stdio::piped())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains(": Audio:"))
        .filter_map(|l| {
            let id = l.split("[0x").nth(1)?.split(']').next()?;
            u32::from_str_radix(id, 16).ok()
        })
        .collect()
}

/// The movies' sound is 5.1 in tracks: front left and right (track 0),
/// the centre (1), the low frequencies (2) and the rear pair (3). In the
/// cut-scenes the centre, which carries the dialogue, comes in one track
/// per language instead, numbered from 2001 (2001 and 2003 to 2006 in
/// the PC copy): 2001 is taken as English (inferred: the lowest, as in
/// EA's language numbering with English first; the others are French,
/// German, Italian and Spanish in some order). Returns the `ffmpeg` filter
/// that puts the tracks together as 5.1 (downmixed to stereo after),
/// or None for a movie with a single sound track.
pub fn sound_mix(tracks: &[u32]) -> Option<String> {
    if tracks.len() < 2 {
        return None;
    }
    let centre = if tracks.contains(&DIALOGUE) {
        Some(DIALOGUE)
    } else {
        tracks.contains(&1).then_some(1)
    };
    let mut inputs = String::new();
    let mut channels = Vec::new();
    let mut at = 0;
    let mut add = |id: u32, names: &[&str], inputs: &mut String| {
        inputs.push_str(&format!("[0:a:i:{id}]"));
        for (k, name) in names.iter().enumerate() {
            channels.push(format!("{name}=c{}", at + k));
        }
        at += names.len();
    };
    if tracks.contains(&0) {
        add(0, &["FL", "FR"], &mut inputs);
    }
    if let Some(c) = centre {
        add(c, &["FC"], &mut inputs);
    }
    if tracks.contains(&2) {
        add(2, &["LFE"], &mut inputs);
    }
    if tracks.contains(&3) {
        add(3, &["BL", "BR"], &mut inputs);
    }
    let n = inputs.matches('[').count();
    if n < 2 {
        return None;
    }
    Some(format!(
        "{inputs}amerge=inputs={n},pan=5.1|{}[mix]",
        channels.join("|")
    ))
}

/// The cut-scenes' English dialogue track (see [`sound_mix`]).
pub const DIALOGUE: u32 = 2001;

pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up over the game.
        cmd.creation_flags(0x0800_0000);
    }
    cmd.stdin(Stdio::null()).stderr(Stdio::null())
}
