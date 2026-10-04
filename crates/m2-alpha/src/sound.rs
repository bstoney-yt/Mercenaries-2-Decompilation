//! The game's sound: an output device fed by the engine's mixer, the
//! island's ambient sounds, music on demand, and the fight's gunshots and
//! blasts.
//!
//! Which sound in a bank is a shot or a blast is not known (sounds are
//! named by hash), so they are picked by measuring, as in the viewer: the
//! rifle bank's loudest short mono sound, the RPG bank's loudest short one,
//! and the artillery bank's loudest 1.5-3 s mono sound.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use m2_engine::audio::{Play, SoundLibrary};
use m2_engine::{Install, Mixer, SoundBuffer, Streams, VoiceId};

use crate::combat::Sound;
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
    /// Rifle shot, rocket launch, explosion.
    fight: [Option<SoundBuffer>; 3],
    /// Short sounds of the town (`amb_citymed`) and the jungle
    /// (`amb_jungle`) played now and then around the listener.
    ambient: [Vec<SoundBuffer>; 2],
    /// Seconds to the next one.
    ambient_wait: f32,
    seed: u64,
}

/// Picks the loudest sound in `bank` passing `keep`.
fn pick(
    library: &SoundLibrary,
    bank: &str,
    keep: impl Fn(&SoundBuffer) -> bool,
) -> Option<SoundBuffer> {
    library
        .bank(bank)
        .iter()
        .filter(|s| s.frames() > 0 && keep(s))
        .max_by(|a, b| a.loudness().total_cmp(&b.loudness()))
        .cloned()
}

/// Ambient one-shots no longer than this, seconds (the banks' longer
/// sounds are loops).
const AMBIENT_LONGEST: f32 = 8.0;

/// The fight's sounds and the ambient one-shots.
fn bank_sounds(install: &Install) -> ([Option<SoundBuffer>; 3], [Vec<SoundBuffer>; 2]) {
    let library = install.open_world().and_then(|mut source| {
        SoundLibrary::load(
            &mut source,
            &[
                "wpn_assaultrifle",
                "wpn_rpg",
                "support_artillery",
                "amb_citymed",
                "amb_jungle",
            ],
        )
    });
    match library {
        Ok(l) => {
            let short = |bank: &str| -> Vec<SoundBuffer> {
                l.bank(bank)
                    .iter()
                    .filter(|s| s.frames() > 0 && s.seconds() <= AMBIENT_LONGEST)
                    .cloned()
                    .collect()
            };
            (
                [
                    pick(&l, "wpn_assaultrifle", |s| {
                        s.channels == 1 && s.seconds() < 0.6
                    }),
                    pick(&l, "wpn_rpg", |s| s.seconds() < 1.5),
                    pick(&l, "support_artillery", |s| {
                        s.channels == 1 && (1.5..=3.0).contains(&s.seconds())
                    }),
                ],
                [short("amb_citymed"), short("amb_jungle")],
            )
        }
        Err(e) => {
            eprintln!("sound: no weapon or ambient sounds ({e})");
            Default::default()
        }
    }
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
        let (fight, ambient) = install.map(bank_sounds).unwrap_or_default();
        // The recorded ambience (ambience.pws) isn't played: which of its
        // tracks is which place is not known, and the first is a tank
        // rolling by. The banks' short sounds stand in, as in the viewer.
        Self {
            out,
            streams,
            music: None,
            next_track: 0,
            decoded: channel(),
            fight,
            ambient,
            ambient_wait: 3.0,
            seed: 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Counts down to the next ambient sound: true when one is due.
    pub fn ambient_due(&mut self, dt: f32) -> bool {
        self.ambient_wait -= dt;
        self.ambient_wait <= 0.0
    }

    /// Plays a town or jungle sound 10 to 40 m from `listener`, and waits
    /// 3 to 9 s for the next.
    pub fn play_ambient(&mut self, town: bool, listener: Vec3) {
        self.ambient_wait = 3.0 + self.random() * 6.0;
        let n = self.ambient[usize::from(!town)].len();
        if n == 0 {
            return;
        }
        let pick = ((self.random() * n as f32) as usize).min(n - 1);
        let angle = self.random() * std::f32::consts::TAU;
        let distance = 10.0 + self.random() * 30.0;
        let at = listener + Vec3::new(angle.cos() * distance, 2.0, angle.sin() * distance);
        let buffer = self.ambient[usize::from(!town)][pick].clone();
        self.out.with(|m| m.play(Play::at(buffer, at).gain(0.5)));
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

    /// Plays the fight's sounds where they happened.
    pub fn play_fight(&mut self, sounds: &[Sound]) {
        for sound in sounds {
            let (i, at, gain) = match *sound {
                Sound::Shot(at) => (0, at, 0.9),
                Sound::Launch(at) => (1, at, 1.0),
                Sound::Explosion(at) => (2, at, 1.4),
            };
            if let Some(buffer) = self.fight[i].clone() {
                self.out.with(|m| m.play(Play::at(buffer, at).gain(gain)));
            }
        }
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
