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

use std::time::{Duration, Instant};

use m2_engine::audio::{Falloff, Play, SoundLibrary};
use m2_engine::{Install, Mixer, SoundBuffer, Streams, VoiceId, Voices};

use crate::combat::{Remark, Sound};
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
    /// Each engine bank's loop, by bank name.
    engines: Vec<(&'static str, SoundBuffer)>,
    /// The engine running now: its bank and voice.
    engine: Option<(&'static str, VoiceId)>,
    /// Seconds to the next one.
    ambient_wait: f32,
    seed: u64,
    /// The characters' voices, once read (on a thread: `English.wad`).
    voices: Option<Arc<Voices>>,
    voices_loading: Option<Receiver<Voices>>,
    /// Decoded lines waiting to play: where (`None` flat) and how.
    said: (Sender<Spoken>, Receiver<Spoken>),
    /// When each line now playing or decoding ends.
    speaking: Vec<Instant>,
    /// When Jennifer may speak again.
    quiet_until: Instant,
}

/// A decoded voice line: the sound, where it is said (`None` flat: the
/// player's own), its falloff and volume.
type Spoken = (SoundBuffer, Option<Vec3>, Falloff, f32);

/// Voice sets, by the name hash of their name. The soldiers' and
/// civilians' sets are named only by hash, and were told apart by their
/// subtitles: two sets of Venezuelan army soldiers (so not everyone sounds
/// alike) and civilians complaining.
pub const VZ_SOLDIERS: [u32; 2] = [0xcf4f_8b9c, 0x6cff_c547];
pub const CIVILIANS: u32 = 0x910e_b574;
/// Jennifer's voice set, `vo_jen`.
pub const JENNIFER: u32 = 0x7f74_6c16;
/// Her lines for what happens to her, by cue (a line, or a group of
/// variants to pick from), chosen by reading their subtitles.
pub fn jennifer_cues(remark: Remark) -> &'static [u32] {
    match remark {
        // Two about dead Venezuelan troops (everyone she fights here) and
        // two said to anyone.
        Remark::Kill => &[0xdcad_44ab, 0x02af_bf14, 0x5157_a743, 0x8eee_b3db],
        // There is no line for wrecking a vehicle: one of the kill lines
        // that suits it.
        Remark::Wreck => &[0x8eee_b3db],
        // Her group of pain grunts, and short lines about being hit.
        Remark::Hurt => &[
            0x559f_e5ef,
            0x3aab_17ee,
            0x8dae_e9a4,
            0x85aa_5fde,
            0x8fbb_b8bd,
            0x339d_71d2,
            0x0db4_3052,
        ],
        // Her group of death cries.
        Remark::Down => &[0x33e7_b94d],
        // Two of her lines for taking someone's car.
        Remark::Vehicle => &[0x84b6_cf3b, 0x88d4_409c],
    }
}
/// How often she says something about it (she doesn't remark on every
/// hit), and the least time between her remarks, seconds.
fn remark_chance(remark: Remark) -> f32 {
    match remark {
        Remark::Kill => 0.3,
        Remark::Wreck | Remark::Vehicle => 0.8,
        Remark::Hurt => 0.3,
        Remark::Down => 1.0,
    }
}
const REMARK_GAP: f32 = 10.0;
/// Shouts are the sets' short lines said where the speaker stands.
const SHOUT_LONGEST: f32 = 3.5;
/// Most lines said at once.
const MAX_SPEAKING: usize = 2;

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

/// Engine banks, and which vehicles use them (by kind and label words,
/// first match; as the browser viewer picks them).
const ENGINES: [(&str, &str); 7] = [
    ("helicopter", "veh_helicoptermedium"),
    ("hmmwv", "veh_jeep"),
    ("tank", "veh_tankbig"),
    ("apc", "veh_tankmed"),
    ("semi|bus|truck", "veh_semi"),
    ("global_veh", "veh_8cylinderold"),
    ("", "veh_4cylindernew"),
];

/// The engine bank for a vehicle of `kind` named `label`.
pub fn engine_bank(kind: &str, label: &str) -> &'static str {
    let key = format!("{kind} {label}").to_lowercase();
    ENGINES
        .iter()
        .find(|(words, _)| words.is_empty() || words.split('|').any(|w| key.contains(w)))
        .map_or("veh_4cylindernew", |e| e.1)
}

/// Sounds read from the banks: the fight's, the ambient one-shots and the
/// engine loops.
type BankSounds = (
    [Option<SoundBuffer>; 3],
    [Vec<SoundBuffer>; 2],
    Vec<(&'static str, SoundBuffer)>,
);

/// The fight's sounds, the ambient one-shots and the engine loops.
fn bank_sounds(install: &Install) -> BankSounds {
    let library = install.open_world().and_then(|mut source| {
        SoundLibrary::load(
            &mut source,
            &[
                "wpn_assaultrifle",
                "wpn_rpg",
                "support_artillery",
                "amb_citymed",
                "amb_jungle",
                "veh_helicoptermedium",
                "veh_jeep",
                "veh_tankbig",
                "veh_tankmed",
                "veh_semi",
                "veh_8cylinderold",
                "veh_4cylindernew",
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
                // Engine loops run 2-8 s; longer ones are rev sweeps.
                ENGINES
                    .iter()
                    .filter_map(|&(_, bank)| {
                        Some((
                            bank,
                            pick(&l, bank, |s| (2.0..=8.0).contains(&s.seconds()))?,
                        ))
                    })
                    .collect(),
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
        let (fight, ambient, engines) = install.map(bank_sounds).unwrap_or_default();
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
            engines,
            engine: None,
            ambient_wait: 3.0,
            seed: 0x9e37_79b9_7f4a_7c15,
            voices: None,
            voices_loading: install.map(|install| {
                let install = install.clone();
                let (tx, rx) = channel();
                std::thread::spawn(move || match Voices::load(&install) {
                    Ok(v) => {
                        eprintln!(
                            "voices: {} sets, {} lines",
                            v.sets.len(),
                            v.index.streams.len()
                        );
                        let _ = tx.send(v);
                    }
                    Err(e) => eprintln!("voices: {e}"),
                });
                rx
            }),
            said: channel(),
            speaking: Vec::new(),
            quiet_until: Instant::now(),
        }
    }

    /// Says a short line of voice set `set` where `at` is (`None`: flat,
    /// the player's own), unless enough people are talking already.
    pub fn shout(&mut self, set: u32, at: Option<Vec3>) {
        let now = Instant::now();
        self.speaking.retain(|end| *end > now);
        if self.speaking.len() >= MAX_SPEAKING {
            return;
        }
        let Some(voices) = self.voices.clone() else {
            return;
        };
        let Some(table) = voices.set(set) else {
            return;
        };
        let lines: Vec<_> = table
            .lines
            .iter()
            .filter(|l| {
                l.is_positional()
                    && voices
                        .index
                        .streams
                        .get(l.stream as usize)
                        .is_some_and(|s| s.seconds() <= SHOUT_LONGEST)
            })
            .cloned()
            .collect();
        if lines.is_empty() {
            return;
        }
        let pick = ((self.random() * lines.len() as f32) as usize).min(lines.len() - 1);
        let line = lines[pick].clone();
        let falloff = Falloff {
            near: line.min_distance.max(1.0),
            far: line.max_distance.max(line.min_distance + 1.0),
        };
        self.speak(voices, line.stream, at, falloff, line.volume);
    }

    /// Jennifer says something about `remark`, now and then: a cue from
    /// [`jennifer_cues`] (one of a group's variants), flat, as her own.
    pub fn remark(&mut self, remark: Remark) {
        let now = Instant::now();
        // Going down cuts in; anything else waits its turn.
        if (now < self.quiet_until && remark != Remark::Down)
            || self.random() >= remark_chance(remark)
        {
            return;
        }
        let Some(voices) = self.voices.clone() else {
            return;
        };
        let Some(table) = voices.set(JENNIFER) else {
            return;
        };
        let streams: Vec<(u32, f32)> = jennifer_cues(remark)
            .iter()
            .filter_map(|&cue| {
                let volume = table
                    .lines
                    .iter()
                    .find(|l| l.cue == cue)
                    .map(|l| l.volume)
                    .or_else(|| table.groups.iter().find(|g| g.cue == cue).map(|g| g.volume))?;
                Some((table.streams(cue)?, volume))
            })
            .flat_map(|(streams, volume)| streams.into_iter().map(move |s| (s, volume)))
            .collect();
        if streams.is_empty() {
            return;
        }
        let pick = ((self.random() * streams.len() as f32) as usize).min(streams.len() - 1);
        let (stream, volume) = streams[pick];
        let seconds = voices
            .index
            .streams
            .get(stream as usize)
            .map_or(0.0, |s| s.seconds());
        self.quiet_until = now + Duration::from_secs_f32(seconds + REMARK_GAP);
        eprintln!("voice: Jennifer, {remark:?}, stream {stream}");
        self.speak(voices, stream, None, Falloff::default(), volume);
    }

    /// Decodes voice stream `stream` on a thread and plays it at `at`.
    fn speak(
        &mut self,
        voices: Arc<Voices>,
        stream: u32,
        at: Option<Vec3>,
        falloff: Falloff,
        volume: f32,
    ) {
        let Some(seconds) = voices
            .index
            .streams
            .get(stream as usize)
            .map(|s| s.seconds())
        else {
            return;
        };
        self.speaking
            .push(Instant::now() + Duration::from_secs_f32(seconds + 0.3));
        let tx = self.said.0.clone();
        std::thread::spawn(move || match voices.decode(stream as usize) {
            Ok(b) => drop(tx.send((b, at, falloff, volume.clamp(0.0, 2.0)))),
            Err(e) => eprintln!("voice line {stream}: {e}"),
        });
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

    /// Runs the engine of the vehicle being driven (`None`: on foot):
    /// its bank's loop, pitched up with speed (m/s) towards `top`.
    pub fn engine(&mut self, driving: Option<(&'static str, f32, f32)>) {
        let want = driving.map(|d| d.0);
        if let Some((bank, id)) = self.engine
            && Some(bank) != want
        {
            self.out.with(|m| m.stop(id));
            self.engine = None;
        }
        let Some((bank, speed, top)) = driving else {
            return;
        };
        if self.engine.is_none()
            && let Some((_, buffer)) = self.engines.iter().find(|e| e.0 == bank)
        {
            let buffer = buffer.clone();
            let id = self
                .out
                .with(|m| m.play(Play::flat(buffer).looped().gain(0.45)));
            self.engine = Some((bank, id));
        }
        if let Some((_, id)) = self.engine {
            let pitch = 0.75 + speed.abs() / top.max(1.0);
            self.out.with(|m| m.set_pitch(id, pitch));
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
        if let Some(rx) = &self.voices_loading
            && let Ok(v) = rx.try_recv()
        {
            self.voices = Some(Arc::new(v));
            self.voices_loading = None;
        }
        while let Ok((buffer, at, falloff, gain)) = self.said.1.try_recv() {
            let play = match at {
                Some(at) => Play {
                    falloff,
                    ..Play::at(buffer, at)
                },
                None => Play::flat(buffer),
            };
            self.out.with(|m| m.play(play.gain(gain)));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_go_by_kind_and_name() {
        assert_eq!(engine_bank("tank", "al_veh_tank_m1"), "veh_tankbig");
        assert_eq!(engine_bank("car", "al_veh_apc_stryker"), "veh_tankmed");
        assert_eq!(engine_bank("truck", "semi_orange"), "veh_semi");
        assert_eq!(
            engine_bank("car", "global_veh_68valiant4door"),
            "veh_8cylinderold"
        );
        assert_eq!(engine_bank("car", "m151"), "veh_4cylindernew");
    }
}
