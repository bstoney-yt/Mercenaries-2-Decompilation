//! The game's sound: an output device fed by the engine's mixer, the
//! island's ambience, and music on demand.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use m2_engine::audio::Play;
use m2_engine::{Install, Mixer, SoundBuffer, Streams, VoiceId};
use m2_math::glam::Vec3;

/// The mixer, and the device stream playing it (none where there is no
/// output: other platforms, or no sound device).
pub struct AudioOut {
    pub mixer: Arc<Mutex<Mixer>>,
    #[cfg(windows)]
    _stream: Option<cpal::Stream>,
}

impl std::fmt::Debug for AudioOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioOut").finish_non_exhaustive()
    }
}

impl AudioOut {
    #[cfg(windows)]
    pub fn start() -> Self {
        match open_device() {
            Ok((mixer, stream)) => Self {
                mixer,
                _stream: Some(stream),
            },
            Err(e) => {
                eprintln!("sound: {e:#}; playing silently");
                Self {
                    mixer: Arc::new(Mutex::new(Mixer::new(48_000))),
                    _stream: None,
                }
            }
        }
    }

    #[cfg(not(windows))]
    pub fn start() -> Self {
        eprintln!("sound: no output on this platform yet; playing silently");
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new(48_000))),
        }
    }

    /// Runs `f` on the mixer (a poisoned lock is taken anyway: the mixer
    /// holds no invariants a panic could break).
    pub fn with<T>(&self, f: impl FnOnce(&mut Mixer) -> T) -> T {
        let mut m = self.mixer.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut m)
    }
}

#[cfg(windows)]
fn open_device() -> anyhow::Result<(Arc<Mutex<Mixer>>, cpal::Stream)> {
    use anyhow::Context as _;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let device = cpal::default_host()
        .default_output_device()
        .context("no sound output device")?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = usize::from(config.channels.max(1));
    let mixer = Arc::new(Mutex::new(Mixer::new(config.sample_rate)));
    let shared = mixer.clone();
    let mut stereo = Vec::new();
    let mut fill = move |frames: usize, mut put: Box<dyn FnMut(usize, f32) + '_>| {
        stereo.resize(frames * 2, 0.0);
        shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mix(&mut stereo);
        for f in 0..frames {
            for c in 0..channels {
                let s = if c < 2 { stereo[f * 2 + c] } else { 0.0 };
                put(f * channels + c, s);
            }
        }
    };
    let error = |e| eprintln!("sound: {e}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            config,
            move |data, _| {
                let frames = data.len() / channels;
                fill(frames, Box::new(|i, s| data[i] = s));
            },
            error,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
            config,
            move |data, _| {
                let frames = data.len() / channels;
                fill(frames, Box::new(|i, s| data[i] = (s * 32767.0) as i16));
            },
            error,
            None,
        )?,
        other => anyhow::bail!("unsupported sample format {other:?}"),
    };
    stream.play()?;
    Ok((mixer, stream))
}

/// Ambience and music.
#[derive(Debug)]
pub struct GameAudio {
    pub out: AudioOut,
    streams: Option<Arc<Streams>>,
    music: Option<VoiceId>,
    next_track: usize,
    decoded: (Sender<Decoded>, Receiver<Decoded>),
}

/// A finished decode: which kind of stream, and its samples.
type Decoded = (&'static str, SoundBuffer);

impl GameAudio {
    /// Opens the output and, with an install, starts the ambience.
    pub fn start(install: Option<&Install>) -> Self {
        let out = AudioOut::start();
        let streams = install.and_then(|install| {
            let loaded = install
                .open_world()
                .and_then(|mut source| Streams::load(install, &mut source));
            match loaded {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    eprintln!("sound: no music or ambience ({e})");
                    None
                }
            }
        });
        let mut audio = Self {
            out,
            streams,
            music: None,
            next_track: 0,
            decoded: channel(),
        };
        audio.decode("ambience", 0);
        audio
    }

    /// Decodes stream `i` of `which` ("music" or "ambience") on a thread.
    fn decode(&mut self, which: &'static str, i: usize) {
        let Some(streams) = self.streams.clone() else {
            return;
        };
        let tx = self.decoded.0.clone();
        std::thread::spawn(move || {
            let index = if which == "music" {
                &streams.music
            } else {
                &streams.ambience
            };
            match streams.decode(index, i) {
                Ok(b) => drop(tx.send((which, b))),
                Err(e) => eprintln!("sound: {which} {i}: {e}"),
            }
        });
    }

    /// Starts the next music track, or stops the music.
    pub fn toggle_music(&mut self) {
        if let Some(id) = self.music.take() {
            self.out.with(|m| m.stop(id));
            return;
        }
        let Some(count) = self.streams.as_ref().map(|s| s.music.streams.len()) else {
            return;
        };
        if count == 0 {
            return;
        }
        let track = self.next_track % count;
        self.next_track += 1;
        eprintln!("music: track {track} of {count}");
        self.decode("music", track);
    }

    /// Starts what finished decoding and moves the listener.
    pub fn update(&mut self, listener: Vec3, forward: Vec3) {
        while let Ok((which, buffer)) = self.decoded.1.try_recv() {
            let id = self.out.with(|m| match which {
                "music" => m.play(Play::flat(buffer).gain(0.6)),
                _ => m.play(Play::flat(buffer).looped().gain(0.5)),
            });
            if which == "music" {
                self.music = Some(id);
            }
        }
        self.out.with(|m| m.set_listener(listener, forward));
    }
}
