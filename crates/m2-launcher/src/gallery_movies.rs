//! The Gallery page's cut-scenes: every movie in the install's
//! `data\Movies`, listed with its length from its Bink header, and played
//! inside the launcher through a local `ffmpeg` (found as `m2-alpha` finds
//! it: `M2_FFMPEG`, beside the program, or the `PATH`;
//! [`m2_engine::bink::ffmpeg`]). The pictures and sound are piped from
//! `ffmpeg` and never written; nothing is extracted or committed.
//!
//! - **The list** (from samples): the `.bik` files and their headers.
//! - **Labels** (**invented**): `01_AOA_J` reads "Cut-scene 1 (AOA)" with
//!   "Jennifer" (the last letter names the hero, `C`/`J`/`M`; `S` none);
//!   the front end's and the logos' movies have our own names
//!   ([`label`]). Files that don't fit show their file name.
//! - **The sound** (from the 360 code, as `m2-alpha` plays it): the
//!   movie's tracks mixed as `m2-alpha` mixes them
//!   ([`m2_engine::bink::sound_mix`]), downmixed to stereo.
//! - **Locks**: saves keep no record of cut-scenes seen, so every movie is
//!   shown unlocked, and the page says so.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, sync_channel};
use std::time::Instant;

use m2_engine::bink::{BinkInfo, audio_tracks, quiet, sound_mix};

/// One movie on offer.
#[derive(Clone, Debug, PartialEq)]
pub struct MovieItem {
    /// The file name without `.bik`.
    pub name: String,
    pub label: String,
    pub hero: Option<&'static str>,
    pub group: &'static str,
    pub info: Option<BinkInfo>,
}

/// A movie file's readable label, the hero it is for and its group
/// (invented).
pub fn label(name: &str) -> (String, Option<&'static str>, &'static str) {
    let lower = name.to_ascii_lowercase();
    let fixed = |l: &str, g| (l.to_owned(), None, g);
    match lower.as_str() {
        "ea" => return fixed("EA logo", "Logos"),
        "pandemic" => return fixed("Pandemic logo", "Logos"),
        "title_logo" => return fixed("Title logo", "Logos"),
        "title_esrb" => return fixed("Rating screen", "Logos"),
        "attract" => return fixed("Attract loop", "Front end"),
        "shell_mainmenu" => return fixed("Main menu backdrop", "Front end"),
        _ => {}
    }
    if let Some(h) = lower.strip_prefix("shell_") {
        let hero = hero_name(h);
        return (
            format!("Hero select: {}", hero.unwrap_or(h)),
            hero,
            "Front end",
        );
    }
    let parts: Vec<&str> = name.split('_').collect();
    if let [n, code, rest @ ..] = parts.as_slice()
        && let Ok(n) = n.parse::<u32>()
    {
        let hero = match rest {
            [h] if h.len() == 1 => hero_name(&h.to_ascii_lowercase()),
            _ => None,
        };
        return (format!("Cut-scene {n} ({code})"), hero, "Cut-scenes");
    }
    (name.to_owned(), None, "Other")
}

fn hero_name(h: &str) -> Option<&'static str> {
    match h {
        "c" | "chris" => Some("Chris"),
        "j" | "jennifer" => Some("Jennifer"),
        "m" | "mattias" => Some("Mattias"),
        _ => None,
    }
}

/// Every movie in `dir` (`data\Movies`): cut-scenes first in order, then
/// the front end's, the logos and anything else.
pub fn list(dir: &Path) -> Vec<MovieItem> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<MovieItem> = read
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bik")) {
                return None;
            }
            let name = p.file_stem()?.to_string_lossy().into_owned();
            let mut head = [0u8; 44];
            let info = std::fs::File::open(&p)
                .and_then(|mut f| f.read_exact(&mut head))
                .ok()
                .and_then(|()| BinkInfo::parse(&head));
            let (label, hero, group) = label(&name);
            Some(MovieItem {
                name,
                label,
                hero,
                group,
                info,
            })
        })
        .collect();
    let rank = |g: &str| match g {
        "Cut-scenes" => 0,
        "Front end" => 1,
        "Logos" => 2,
        _ => 3,
    };
    out.sort_by(|a, b| rank(a.group).cmp(&rank(b.group)).then(a.name.cmp(&b.name)));
    out
}

/// A movie playing: frames from one `ffmpeg`, the sound from another.
pub struct MoviePlayer {
    pub name: String,
    pub info: BinkInfo,
    video: Child,
    audio: Child,
    frames: Receiver<Vec<u8>>,
    sound: Receiver<Vec<i16>>,
    #[cfg(windows)]
    output: Option<cpal::Stream>,
    opened: Instant,
    started: Option<Instant>,
    shown: u64,
    /// The frame up now (RGBA, `info.width` x `info.height`), new since
    /// last taken.
    fresh: Option<Vec<u8>>,
    pub ended: bool,
}

impl std::fmt::Debug for MoviePlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoviePlayer")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Drop for MoviePlayer {
    fn drop(&mut self) {
        let _ = self.video.kill();
        let _ = self.audio.kill();
    }
}

/// Seconds a movie waits for its sound before starting without it
/// (invented; `m2-alpha` waits as long).
const SOUND_WAIT: f32 = 4.0;
/// The sound's rate.
const RATE: u32 = 48_000;

impl MoviePlayer {
    pub fn open(ffmpeg: &Path, file: &Path) -> Result<Self, String> {
        let mut head = [0u8; 44];
        std::fs::File::open(file)
            .and_then(|mut f| f.read_exact(&mut head))
            .map_err(|e| format!("{}: {e}", file.display()))?;
        let info = BinkInfo::parse(&head).ok_or("not a Bink movie")?;
        let mut video = quiet(
            Command::new(ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(file)
                .args(["-an", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
                .stdout(Stdio::piped()),
        )
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
        let mut out = video.stdout.take().ok_or("no pipe")?;
        let size = (info.width * info.height * 4) as usize;
        let (tx, frames) = sync_channel(6);
        std::thread::spawn(move || {
            loop {
                let mut frame = vec![0u8; size];
                if out.read_exact(&mut frame).is_err() || tx.send(frame).is_err() {
                    break;
                }
            }
        });
        let mut audio = Command::new(ffmpeg);
        audio.args(["-v", "error", "-i"]).arg(file);
        match sound_mix(&audio_tracks(ffmpeg, file)) {
            Some(mix) => audio.args(["-filter_complex", &mix, "-map", "[mix]"]),
            None => audio.arg("-vn"),
        };
        let mut audio = quiet(
            audio
                .args(["-f", "s16le", "-ac", "2", "-ar"])
                .arg(RATE.to_string())
                .arg("-")
                .stdout(Stdio::piped()),
        )
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
        let mut out = audio.stdout.take().ok_or("no pipe")?;
        let (tx, sound) = sync_channel(1);
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if out.read_to_end(&mut bytes).is_ok() {
                let samples = bytes
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let _ = tx.send(samples);
            }
        });
        Ok(Self {
            name: file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            info,
            video,
            audio,
            frames,
            sound,
            #[cfg(windows)]
            output: None,
            started: None,
            shown: 0,
            fresh: None,
            ended: false,
            opened: Instant::now(),
        })
    }

    /// Moves on to the frame due now; starts the sound once it is decoded
    /// (or after [`SOUND_WAIT`] without it).
    pub fn update(&mut self) {
        if self.started.is_none() {
            match self.sound.try_recv() {
                Ok(samples) => {
                    #[cfg(windows)]
                    {
                        self.output = crate::gallery::output(&samples, 2, RATE)
                            .ok()
                            .map(|(s, _)| s);
                    }
                    #[cfg(not(windows))]
                    let _ = samples;
                    self.started = Some(Instant::now());
                }
                Err(_) if self.opened.elapsed().as_secs_f32() > SOUND_WAIT => {
                    self.started = Some(Instant::now());
                }
                Err(_) => return,
            }
        }
        let Some(start) = self.started else { return };
        let due = (start.elapsed().as_secs_f64() * self.info.fps) as u64 + 1;
        while self.shown < due {
            match self.frames.try_recv() {
                Ok(f) => {
                    self.shown += 1;
                    self.fresh = Some(f);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.ended = true;
                    break;
                }
            }
        }
    }

    pub fn take_frame(&mut self) -> Option<Vec<u8>> {
        self.fresh.take()
    }

    pub fn elapsed(&self) -> f32 {
        self.started.map_or(0.0, |s| s.elapsed().as_secs_f32())
    }
}

/// The movie folder of an install.
pub fn movies_dir(install: &m2_engine::Install) -> PathBuf {
    install.data.join("Movies")
}

/// `rect` fitted inside `room` keeping `w`:`h`, centred.
pub fn letterbox(room: (f32, f32), w: u32, h: u32) -> (f32, f32) {
    let aspect = w.max(1) as f32 / h.max(1) as f32;
    if room.0 / room.1.max(1.0) > aspect {
        (room.1 * aspect, room.1)
    } else {
        (room.0, room.0 / aspect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_name_the_cut_scenes_and_heroes() {
        assert_eq!(
            label("01_AOA_J"),
            ("Cut-scene 1 (AOA)".into(), Some("Jennifer"), "Cut-scenes")
        );
        assert_eq!(
            label("11_SR2_S"),
            ("Cut-scene 11 (SR2)".into(), None, "Cut-scenes")
        );
        assert_eq!(
            label("01_VIK_01"),
            ("Cut-scene 1 (VIK)".into(), None, "Cut-scenes")
        );
        assert_eq!(
            label("shell_mattias"),
            ("Hero select: Mattias".into(), Some("Mattias"), "Front end")
        );
        assert_eq!(label("EA"), ("EA logo".into(), None, "Logos"));
        assert_eq!(label("odd_thing"), ("odd_thing".into(), None, "Other"));
    }

    #[test]
    fn the_folder_is_listed_with_headers() {
        let d = std::env::temp_dir().join(format!("m2-gallery-movies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // A synthetic Bink 1 header: 300 frames, 640x360 at 30/1.
        let mut head = b"BIKi".to_vec();
        for v in [0u32, 300, 0, 300, 640, 360, 30, 1, 0, 0] {
            head.extend(v.to_le_bytes());
        }
        std::fs::write(d.join("02_AOB_C.bik"), &head).unwrap();
        std::fs::write(d.join("EA.bik"), &head).unwrap();
        std::fs::write(
            d.join("01_AOA_M.bik"),
            b"not bink, long enough to read 44 bytes........",
        )
        .unwrap();
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        let l = list(&d);
        let names: Vec<&str> = l.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["01_AOA_M", "02_AOB_C", "EA"]);
        assert_eq!(l[0].info, None);
        assert_eq!(
            l[1].info.map(|i| (i.width, i.height, i.seconds())),
            Some((640, 360, 10.0))
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn movies_fit_their_viewport() {
        assert_eq!(letterbox((1000.0, 1000.0), 1280, 720), (1000.0, 562.5));
        assert_eq!(letterbox((2000.0, 720.0), 1280, 720), (1280.0, 720.0));
    }
}
