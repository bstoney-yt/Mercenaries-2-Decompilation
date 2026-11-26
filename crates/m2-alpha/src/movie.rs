//! The game's full-screen movies (`data\Movies\*.bik`, Bink 1), played
//! from the player's install: the boot logos, the front end's backdrop and
//! the contracts' cut-scenes.
//!
//! Decoding is FFmpeg's (its Bink video and audio decoders), run as a
//! separate `ffmpeg` process that pipes raw frames and samples back; the
//! movies themselves are never copied. Where `ffmpeg` is found:
//! `M2_FFMPEG`, else `ffmpeg.exe` beside m2-alpha, else the `PATH`.
//!
//! # The sound
//!
//! From the 360 code: the movie player (`0x824B6788`) opens the file with
//! Bink's sound tracks 0 to 3 and 2001 plus the language's index
//! (`BinkSetSoundTrack`-style table at `0x82DE88D8`, before the open), and
//! then sets every track's volume to the player's dialog volume (the
//! sound manager's third level, which the options' `audioSetDialogVolume`
//! sets) times 43691 (Bink's 32768 is as made). Bink plays it on its own
//! voices, outside the sound engine, so the scripts'
//! `Sound.SetMasterVolume(0)` before a cut-scene (both of the first two
//! contracts' do it; from the mission log) leaves it alone. Here it plays
//! as [`Category::Movie`], which that master level and the category fades
//! skip, at [`MOVIE_GAIN`].
//!
//! Invented: the sound is decoded whole before it plays; a movie waits up
//! to [`SOUND_WAIT`] for it, and sound decoded later starts where the
//! pictures are, so it stays in step (Bink streams it with the frames).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

pub use m2_engine::bink::{BinkInfo, ffmpeg};
use m2_engine::bink::{audio_tracks, quiet, sound_mix};
use m2_engine::{Category, Install, MOVIE_GAIN, Play, SoundBuffer, VoiceId};

/// The sample rate and channels the movies' sound is decoded to.
const RATE: u32 = 48_000;
/// How long a movie waits for its sound before starting without it.
const SOUND_WAIT: Duration = Duration::from_secs(4);
/// Decoded frames held ahead of the one shown.
const AHEAD: usize = 6;

/// The movie named `name` (`EA`, `01_AOA_J`) in the install's movie
/// folder.
pub fn path(install: &Install, name: &str) -> PathBuf {
    install.data.join("Movies").join(format!("{name}.bik"))
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

        let mut audio = Command::new(ffmpeg);
        audio.args(["-v", "error", "-i"]).arg(&file);
        match sound_mix(&audio_tracks(ffmpeg, &file)) {
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
            Some(t) => {
                // Sound decoded after the pictures started: from where
                // they are.
                if self.voice.is_none()
                    && let Ok(buffer) = self.sound.try_recv()
                {
                    let late = from(&buffer, t.elapsed().as_secs_f64());
                    self.voice = Some(play(sound(late)));
                }
                t
            }
            None => {
                match self.sound.try_recv() {
                    Ok(buffer) => {
                        self.voice = Some(play(sound(buffer)));
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

/// How a movie's sound plays: Bink's, outside the sound engine (see the
/// module's docs).
fn sound(buffer: SoundBuffer) -> Play {
    Play::flat(buffer)
        .category(Category::Movie)
        .gain(MOVIE_GAIN)
}

/// `buffer` from `seconds` in (empty past its end).
fn from(buffer: &SoundBuffer, seconds: f64) -> SoundBuffer {
    let channels = usize::from(buffer.channels.max(1));
    let frame = (seconds.max(0.0) * f64::from(buffer.rate)) as usize;
    let at = (frame * channels).min(buffer.samples.len());
    SoundBuffer {
        samples: buffer.samples[at..].into(),
        ..buffer.clone()
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
    fn the_sound_tracks_are_put_together_as_5_1_with_the_english_dialogue() {
        assert_eq!(
            sound_mix(&[0, 2, 3, 2001, 2003, 2004, 2005, 2006]).as_deref(),
            Some(
                "[0:a:i:0][0:a:i:2001][0:a:i:2][0:a:i:3]amerge=inputs=4,\
                 pan=5.1|FL=c0|FR=c1|FC=c2|LFE=c3|BL=c4|BR=c5[mix]"
            )
        );
        assert_eq!(
            sound_mix(&[0, 1, 2, 3]).as_deref(),
            Some(
                "[0:a:i:0][0:a:i:1][0:a:i:2][0:a:i:3]amerge=inputs=4,\
                 pan=5.1|FL=c0|FR=c1|FC=c2|LFE=c3|BL=c4|BR=c5[mix]"
            )
        );
        assert_eq!(sound_mix(&[0]), None);
    }

    fn stereo(frames: i16) -> SoundBuffer {
        SoundBuffer {
            hash: 0,
            channels: 2,
            rate: 10,
            samples: (0..frames).flat_map(|f| [f, -f]).collect(),
        }
    }

    #[test]
    fn the_sound_plays_as_bink_s_outside_the_scripts_master_level() {
        let p = sound(stereo(4));
        assert_eq!(p.category, Category::Movie);
        assert!((p.gain - 43_691.0 / 32_768.0).abs() < 1e-6);
        assert!(p.position.is_none() && !p.looping);
    }

    #[test]
    fn sound_decoded_late_starts_where_the_pictures_are() {
        let b = stereo(30);
        // A second and a half in at 10 Hz: frame 15 on, both channels.
        let late = from(&b, 1.5);
        assert_eq!(late.frames(), 15);
        assert_eq!(&late.samples[..2], &[15, -15]);
        assert_eq!((late.rate, late.channels), (10, 2));
        assert_eq!(from(&b, 0.0), b);
        assert_eq!(from(&b, 9.0).frames(), 0);
    }

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
