//! The game's full-screen movies (`data\Movies\*.bik`, Bink 1), played
//! from the player's install: the boot logos, the front end's backdrop and
//! the contracts' cut-scenes.
//!
//! Decoding is FFmpeg's (its Bink video and audio decoders), run as a
//! separate `ffmpeg` process that pipes raw frames and samples back; the
//! movies themselves are never copied. Where `ffmpeg` is found:
//! `M2_FFMPEG`, else `ffmpeg.exe` beside m2-alpha, else the `PATH`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use m2_engine::{Install, Play, SoundBuffer, VoiceId};

/// The sample rate and channels the movies' sound is decoded to.
const RATE: u32 = 48_000;
/// How long a movie waits for its sound before starting without it.
const SOUND_WAIT: Duration = Duration::from_secs(4);
/// Decoded frames held ahead of the one shown.
const AHEAD: usize = 6;

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

/// The movie named `name` (`EA`, `01_AOA_J`) in the install's movie
/// folder.
pub fn path(install: &Install, name: &str) -> PathBuf {
    install.data.join("Movies").join(format!("{name}.bik"))
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

fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up over the game.
        cmd.creation_flags(0x0800_0000);
    }
    cmd.stdin(Stdio::null()).stderr(Stdio::null())
}

/// A movie playing.
pub struct Movie {
    pub name: String,
    pub info: BinkInfo,
    video: Child,
    audio: Child,
    frames: Receiver<Vec<u8>>,
    sound: Receiver<SoundBuffer>,
    /// The frame up now (RGBA) and its number.
    pub frame: Option<(u64, Vec<u8>)>,
    /// When the frames started (the sound with them).
    started: Option<Instant>,
    opened: Instant,
    /// The frames ran out.
    drained: bool,
    pub voice: Option<VoiceId>,
    /// Played again from the start when it ends (the front end's
    /// backdrop).
    pub looping: bool,
}

impl std::fmt::Debug for Movie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Movie")
            .field("name", &self.name)
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Movie {
    /// Starts decoding movie `name` from `install` with `ffmpeg`.
    pub fn open(ffmpeg: &Path, install: &Install, name: &str) -> anyhow::Result<Self> {
        let file = path(install, name);
        let mut head = [0u8; 44];
        std::fs::File::open(&file)
            .and_then(|mut f| f.read_exact(&mut head))
            .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let info = BinkInfo::parse(&head)
            .ok_or_else(|| anyhow::anyhow!("{}: not a Bink movie", file.display()))?;

        let mut video = quiet(
            Command::new(ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&file)
                .args(["-an", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
                .stdout(Stdio::piped()),
        )
        .spawn()?;
        let mut out = video.stdout.take().expect("piped");
        let size = (info.width * info.height * 4) as usize;
        let (tx, frames) = sync_channel(AHEAD);
        std::thread::spawn(move || {
            loop {
                let mut frame = vec![0u8; size];
                if out.read_exact(&mut frame).is_err() || tx.send(frame).is_err() {
                    break;
                }
            }
        });

        let mut audio = quiet(
            Command::new(ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&file)
                .args(["-vn", "-f", "s16le", "-ac", "2", "-ar"])
                .arg(RATE.to_string())
                .arg("-")
                .stdout(Stdio::piped()),
        )
        .spawn()?;
        let mut out = audio.stdout.take().expect("piped");
        let (tx, sound) = sync_channel(1);
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if out.read_to_end(&mut bytes).is_ok() && bytes.len() >= 4 {
                let samples: Vec<i16> = bytes
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let _ = tx.send(SoundBuffer {
                    hash: 0,
                    channels: 2,
                    rate: RATE,
                    samples: samples.into(),
                });
            }
        });
        eprintln!(
            "movie: {name} {}x{} {:.2} fps, {:.1} s",
            info.width,
            info.height,
            info.fps,
            info.seconds()
        );
        Ok(Self {
            name: name.to_owned(),
            info,
            video,
            audio,
            frames,
            sound,
            frame: None,
            started: None,
            opened: Instant::now(),
            drained: false,
            voice: None,
            looping: false,
        })
    }

    /// Moves on to the frame due now. Its sound, once decoded, is handed
    /// to `play` (which gives back its voice) the moment the frames start.
    /// Returns whether the movie is over.
    pub fn update(&mut self, mut play: impl FnMut(Play) -> VoiceId) -> bool {
        let started = match self.started {
            Some(t) => t,
            None => {
                match self.sound.try_recv() {
                    Ok(buffer) => {
                        self.voice = Some(play(Play::flat(buffer).ui()));
                    }
                    // No sound (yet): wait a little for it.
                    Err(TryRecvError::Empty) if self.opened.elapsed() < SOUND_WAIT => {
                        return false;
                    }
                    Err(_) => {}
                }
                let now = Instant::now();
                self.started = Some(now);
                now
            }
        };
        let due = (started.elapsed().as_secs_f64() * self.info.fps) as u64;
        loop {
            if self.frame.as_ref().is_some_and(|(n, _)| *n >= due) {
                break;
            }
            match self.frames.try_recv() {
                Ok(f) => {
                    let n = self.frame.as_ref().map_or(0, |(n, _)| n + 1);
                    self.frame = Some((n, f));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.drained = true;
                    break;
                }
            }
        }
        // Over once every frame has shown and its time is up (or, should
        // the decoder have stopped early, a second after it did).
        let over_time = started.elapsed().as_secs_f64() > self.info.seconds() + 0.25;
        self.drained && (over_time || due > u64::from(self.info.frames))
    }
}

impl Movie {
    /// Waits for the next frame (for pictures taken offscreen).
    pub fn next_frame(&mut self) -> Option<&[u8]> {
        let f = self.frames.recv().ok()?;
        let n = self.frame.as_ref().map_or(0, |(n, _)| n + 1);
        self.frame = Some((n, f));
        self.frame.as_ref().map(|(_, f)| f.as_slice())
    }

    /// Waits for the decoded sound: its length in seconds, if it has any.
    pub fn sound_seconds(&mut self) -> Option<f32> {
        self.sound
            .recv_timeout(Duration::from_secs(60))
            .ok()
            .map(|b| b.seconds())
    }
}

impl Drop for Movie {
    fn drop(&mut self) {
        let _ = self.video.kill();
        let _ = self.audio.kill();
        let _ = self.video.wait();
        let _ = self.audio.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_bink_header() {
        let mut head = vec![0u8; 44];
        head[..4].copy_from_slice(b"BIKi");
        head[8..12].copy_from_slice(&300u32.to_le_bytes());
        head[20..24].copy_from_slice(&1280u32.to_le_bytes());
        head[24..28].copy_from_slice(&720u32.to_le_bytes());
        head[28..32].copy_from_slice(&30000u32.to_le_bytes());
        head[32..36].copy_from_slice(&1001u32.to_le_bytes());
        let info = BinkInfo::parse(&head).unwrap();
        assert_eq!((info.width, info.height, info.frames), (1280, 720, 300));
        assert!((info.fps - 29.97).abs() < 0.01);
        assert!((info.seconds() - 10.01).abs() < 0.01);
        head[0] = b'X';
        assert_eq!(BinkInfo::parse(&head), None);
    }
}
