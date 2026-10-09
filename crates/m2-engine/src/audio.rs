//! Sound: the game's sound banks and streamed music and ambience, mixed in
//! software.
//!
//! [`Mixer`] holds the playing voices and fills an interleaved stereo
//! buffer for whatever audio device the game opens (the device is the
//! game's concern; this module has no platform code). Voices play a
//! [`SoundBuffer`] once or looped, at a pitch, either flat (music, the HUD)
//! or placed in the world, where they fade with distance and pan around
//! the listener.
//!
//! [`SoundLibrary`] reads banks by the name of the block they sit in
//! (`wpn_assaultrifle`, `veh_jeep`, `amb_citymed`, ...), and [`Streams`]
//! decodes tracks from `data/Audios/music.pws` and `ambience.pws`.
//!
//! # Provenance
//!
//! Clean-room. Formats as `m2_assets::sound` decodes them; the mixing,
//! falloff and panning are ordinary game-audio practice, not the game's.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;

use m2_assets::sound::{SOUND_BANK_TYPE, decode_xbox_adpcm};
use m2_assets::voice::CUE_TABLE_TYPE;
use m2_assets::{BlockContents, CueTable, SoundBank, StreamIndex, name_hash};
use m2_math::glam::Vec3;

use crate::assets::{AssetError, AssetSource, Install};

/// Decoded 16-bit samples, channels interleaved.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundBuffer {
    pub hash: u32,
    pub channels: u16,
    pub rate: u32,
    pub samples: Arc<[i16]>,
}

impl SoundBuffer {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }

    /// Root-mean-square level, 0 to 1.
    pub fn loudness(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self
            .samples
            .iter()
            .map(|&s| (f64::from(s) / 32768.0).powi(2))
            .sum();
        (sum / self.samples.len() as f64).sqrt() as f32
    }

    /// Level of each tenth of the sound, dB below full scale (silence is
    /// -100).
    pub fn tenths_db(&self) -> Vec<f32> {
        let n = self.frames();
        let tenth = (n / 10).max(1);
        (0..10)
            .map(|k| self.window_db(k * tenth, ((k + 1) * tenth).min(n)))
            .collect()
    }

    /// Level of frames `from..to`, dB below full scale (all channels).
    fn window_db(&self, from: usize, to: usize) -> f32 {
        let ch = usize::from(self.channels.max(1));
        let samples = self.samples.get(from * ch..to * ch).unwrap_or_default();
        if samples.is_empty() {
            return -100.0;
        }
        let sum: f64 = samples
            .iter()
            .map(|&s| (f64::from(s) / 32768.0).powi(2))
            .sum();
        let rms = (sum / samples.len() as f64).sqrt();
        if rms <= 1e-5 {
            -100.0
        } else {
            (20.0 * rms.log10()) as f32
        }
    }

    /// Whether its level holds steady, as an engine's loop does: from
    /// tenth to tenth, and over 0.1 s windows (between their quietest and
    /// loudest tenth, so single clicks don't count).
    pub fn is_steady(&self) -> bool {
        let tenths = self.tenths_db();
        let (lo, hi) = tenths
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &d| (lo.min(d), hi.max(d)));
        if hi - lo > STEADY_TENTHS {
            return false;
        }
        let w = (self.rate as usize / 10).max(1);
        let mut windows: Vec<f32> = (0..self.frames() / w)
            .map(|k| self.window_db(k * w, (k + 1) * w))
            .collect();
        if windows.len() < 10 {
            return false;
        }
        windows.sort_by(f32::total_cmp);
        let (p10, p90) = (windows[windows.len() / 10], windows[windows.len() * 9 / 10]);
        p90 - p10 <= STEADY_WINDOWS
    }

    /// Zero crossings a second, first channel: high for hiss and whine,
    /// low for a rumble.
    pub fn crossing_rate(&self) -> f32 {
        let ch = usize::from(self.channels.max(1));
        let n = self.frames();
        if n < 2 {
            return 0.0;
        }
        let crossings = (1..n)
            .filter(|&i| (self.samples[(i - 1) * ch] < 0) != (self.samples[i * ch] < 0))
            .count();
        crossings as f32 / self.seconds().max(1e-3)
    }

    /// The rate of the strongest repeating pattern in its middle second,
    /// Hz, between 20 and 400 Hz (an engine's firing rate), if one stands
    /// out: the period whose stretch of sound most resembles the next, the
    /// shortest of those nearly as alike as the best (not a multiple of it).
    pub fn fundamental(&self) -> Option<f32> {
        // Block averages down to about 4 kHz: a crude low-pass that keeps
        // the engine's beat and drops its hiss.
        let ch = usize::from(self.channels.max(1));
        let step = ((self.rate as f32 / 4000.0).round() as usize).max(1);
        let rate = self.rate as f32 / step as f32;
        let n = self.frames() / step;
        let keep = (rate as usize).min(n);
        let from = (n - keep) / 2;
        let mut x: Vec<f32> = (from..from + keep)
            .map(|i| {
                let block = &self.samples[i * step * ch..(i + 1) * step * ch];
                block.iter().map(|&s| f32::from(s)).sum::<f32>() / block.len() as f32
            })
            .collect();
        let mean = x.iter().sum::<f32>() / x.len().max(1) as f32;
        for v in &mut x {
            *v -= mean;
        }
        let (lo, hi) = ((rate / 400.0) as usize, (rate / 20.0) as usize);
        if lo < 1 || x.len() < hi * 3 {
            return None;
        }
        let alike = |lag: usize| -> f32 {
            let (mut xy, mut xx, mut yy) = (0.0f64, 0.0f64, 0.0f64);
            for i in 0..x.len() - lag {
                let (a, b) = (f64::from(x[i]), f64::from(x[i + lag]));
                xy += a * b;
                xx += a * a;
                yy += b * b;
            }
            if xx <= 0.0 || yy <= 0.0 {
                0.0
            } else {
                (xy / (xx * yy).sqrt()) as f32
            }
        };
        let r: Vec<f32> = (lo..=hi).map(alike).collect();
        let best = r.iter().copied().fold(f32::MIN, f32::max);
        if best < 0.3 {
            return None;
        }
        let k = (1..r.len() - 1)
            .find(|&k| r[k] >= best * 0.85 && r[k] >= r[k - 1] && r[k] >= r[k + 1])?;
        Some(rate / (lo + k) as f32)
    }

    fn frame(&self, i: usize) -> (f32, f32) {
        let ch = usize::from(self.channels.max(1));
        let at = i * ch;
        let s = |k: usize| f32::from(self.samples.get(at + k).copied().unwrap_or(0)) / 32768.0;
        if ch == 1 { (s(0), s(0)) } else { (s(0), s(1)) }
    }
}

/// Distances, metres, for placed sounds: full volume within `near`,
/// falling off as `near / distance` beyond it, silent past `far`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Falloff {
    pub near: f32,
    pub far: f32,
}

impl Default for Falloff {
    fn default() -> Self {
        Self {
            near: 4.0,
            far: 250.0,
        }
    }
}

impl Falloff {
    pub fn gain(&self, distance: f32) -> f32 {
        if distance >= self.far {
            return 0.0;
        }
        let g = self.near / distance.max(self.near);
        // Fade the last fifth to zero so sounds don't cut out.
        let edge = ((self.far - distance) / (self.far * 0.2)).clamp(0.0, 1.0);
        g * edge
    }
}

/// Which of the game's sound categories a sound is in. The scripts fade
/// and pitch them by name (`MrxSoundCategories` sets): `non_ui` is the
/// world's own sounds, and dialogue, barks and music go separately (the
/// fanfare fades `non_ui` and `vo` both, and the survival rescue pitches
/// barks less than the world; inferred, the engine's tree not decoded).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Category {
    /// `non_ui`: everything placed in the world, engines, guns, ambience.
    #[default]
    World,
    /// `chatter`: people's barks.
    Chatter,
    /// `music`.
    Music,
    /// `vo`: scripted dialogue, the radio's included.
    Vo,
    /// The interface's sounds, which no script fades.
    Ui,
}

impl Category {
    const ALL: usize = 5;

    /// The category a script names (`non_ui`, `chatter`, ...), if one we
    /// play.
    pub fn named(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "non_ui" => Self::World,
            "chatter" => Self::Chatter,
            "music" => Self::Music,
            "vo" => Self::Vo,
            "ui" => Self::Ui,
            _ => return None,
        })
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A value gliding to a goal at a steady rate.
#[derive(Clone, Copy, Debug)]
struct Glide {
    now: f32,
    goal: f32,
    /// Change per second.
    rate: f32,
}

impl Glide {
    const ONE: Self = Self {
        now: 1.0,
        goal: 1.0,
        rate: f32::MAX,
    };

    fn to(&mut self, goal: f32, seconds: f32) {
        self.goal = goal;
        self.rate = if seconds > 1e-3 {
            (goal - self.now).abs() / seconds
        } else {
            f32::MAX
        };
    }

    fn step(&mut self, seconds: f32) {
        let step = self.rate * seconds;
        self.now += (self.goal - self.now).clamp(-step, step);
    }
}

/// How to play a sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Play {
    pub buffer: SoundBuffer,
    pub gain: f32,
    /// Playback rate multiplier (1 plays as recorded).
    pub pitch: f32,
    pub looping: bool,
    /// Where it is in the world; `None` plays it flat.
    pub position: Option<Vec3>,
    pub falloff: Falloff,
    /// Its category, which [`Mixer::fade`] and [`Mixer::pitch`] work on.
    pub category: Category,
}

impl Play {
    pub fn flat(buffer: SoundBuffer) -> Self {
        Self {
            buffer,
            gain: 1.0,
            pitch: 1.0,
            looping: false,
            position: None,
            falloff: Falloff::default(),
            category: Category::World,
        }
    }

    pub fn at(buffer: SoundBuffer, position: Vec3) -> Self {
        Self {
            position: Some(position),
            ..Self::flat(buffer)
        }
    }

    pub fn looped(self) -> Self {
        Self {
            looping: true,
            ..self
        }
    }

    pub fn gain(self, gain: f32) -> Self {
        Self { gain, ..self }
    }

    /// Marks it an interface sound, which no script fades.
    pub fn ui(self) -> Self {
        self.category(Category::Ui)
    }

    pub fn category(self, category: Category) -> Self {
        Self { category, ..self }
    }
}

/// A playing sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId(u64);

#[derive(Debug)]
struct Voice {
    id: VoiceId,
    play: Play,
    /// Position in source frames.
    cursor: f64,
    /// Gains last used, ramped towards each buffer to avoid clicks.
    current: (f32, f32),
}

/// Output frames over which a gain change glides.
const RAMP_FRAMES: usize = 64;

/// Mixes voices to interleaved stereo `f32`.
#[derive(Debug)]
pub struct Mixer {
    rate: u32,
    voices: Vec<Voice>,
    next: u64,
    listener: Vec3,
    /// The listener's right, horizontal.
    right: Vec3,
    pub master: f32,
    /// Each category's level and pitch, glided by [`Self::fade`] and
    /// [`Self::pitch`] (the scripts duck the world under a radio
    /// conversation and slow it in the survival rescue).
    levels: [Glide; Category::ALL],
    pitches: [Glide; Category::ALL],
    /// Most voices played at once; the quietest are dropped past it.
    pub max_voices: usize,
}

impl Mixer {
    /// A mixer producing `rate` Hz.
    pub fn new(rate: u32) -> Self {
        Self {
            rate: rate.max(1),
            voices: Vec::new(),
            next: 1,
            listener: Vec3::ZERO,
            right: Vec3::X,
            master: 0.8,
            levels: [Glide::ONE; Category::ALL],
            pitches: [Glide::ONE; Category::ALL],
            max_voices: 64,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Glides `category`'s sounds to `level` over `seconds`
    /// (`Sound.FadeCategoryDown`/`Up`).
    pub fn fade(&mut self, category: Category, level: f32, seconds: f32) {
        self.levels[category.index()].to(level.clamp(0.0, 1.0), seconds);
    }

    /// Glides `category`'s sounds to `pitch` times their own over
    /// `seconds` (`Sound.PitchCategoryActivate`/`Deactivate`).
    pub fn pitch(&mut self, category: Category, pitch: f32, seconds: f32) {
        self.pitches[category.index()].to(pitch.clamp(0.05, 4.0), seconds);
    }

    /// The level `category`'s sounds play at now.
    pub fn level(&self, category: Category) -> f32 {
        self.levels[category.index()].now
    }

    pub fn play(&mut self, play: Play) -> VoiceId {
        let id = VoiceId(self.next);
        self.next += 1;
        if self.voices.len() >= self.max_voices
            && let Some(quietest) = self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| !v.play.looping)
                .min_by(|a, b| self.loudness(a.1).total_cmp(&self.loudness(b.1)))
                .map(|(i, _)| i)
        {
            self.voices.swap_remove(quietest);
        }
        self.voices.push(Voice {
            id,
            play,
            cursor: 0.0,
            current: (0.0, 0.0),
        });
        id
    }

    pub fn stop(&mut self, id: VoiceId) {
        self.voices.retain(|v| v.id != id);
    }

    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.voices.iter().any(|v| v.id == id)
    }

    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    fn voice_mut(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id)
    }

    pub fn set_position(&mut self, id: VoiceId, position: Vec3) {
        if let Some(v) = self.voice_mut(id) {
            v.play.position = Some(position);
        }
    }

    pub fn set_pitch(&mut self, id: VoiceId, pitch: f32) {
        if let Some(v) = self.voice_mut(id) {
            v.play.pitch = pitch;
        }
    }

    pub fn set_gain(&mut self, id: VoiceId, gain: f32) {
        if let Some(v) = self.voice_mut(id) {
            v.play.gain = gain;
        }
    }

    /// Where the listener is and which way they face.
    pub fn set_listener(&mut self, position: Vec3, forward: Vec3) {
        self.listener = position;
        // Right-handed, +y up: facing +z, right is -x.
        let f = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::Z);
        self.right = Vec3::new(-f.z, 0.0, f.x);
    }

    /// Left and right gain for `play` from the listener.
    fn gains(&self, play: &Play) -> (f32, f32) {
        let g = play.gain * self.master * self.level(play.category);
        let Some(at) = play.position else {
            return (g, g);
        };
        let offset = at - self.listener;
        let distance = offset.length();
        let g = g * play.falloff.gain(distance);
        // Constant-power pan, kept off the extremes so one ear never goes
        // silent.
        let pan = if distance > 1e-3 {
            (offset.dot(self.right) / distance).clamp(-1.0, 1.0) * 0.8
        } else {
            0.0
        };
        let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        (
            g * angle.cos() * std::f32::consts::SQRT_2,
            g * angle.sin() * std::f32::consts::SQRT_2,
        )
    }

    fn loudness(&self, v: &Voice) -> f32 {
        let (l, r) = self.gains(&v.play);
        l.max(r)
    }

    /// Adds every voice into `out` (interleaved stereo, cleared first) and
    /// drops voices that finished.
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }
        let seconds = frames as f32 / self.rate as f32;
        for g in self.levels.iter_mut().chain(&mut self.pitches) {
            g.step(seconds);
        }
        let pitches = self.pitches.map(|g| g.now);
        let gains: Vec<(f32, f32)> = self.voices.iter().map(|v| self.gains(&v.play)).collect();
        for (v, target) in self.voices.iter_mut().zip(gains) {
            let len = v.play.buffer.frames();
            if len == 0 {
                v.cursor = f64::MAX;
                continue;
            }
            let step = f64::from(v.play.buffer.rate) / f64::from(self.rate)
                * f64::from(v.play.pitch.max(0.01) * pitches[v.play.category.index()]);
            let (mut gl, mut gr) = v.current;
            // Gain changes glide over a few milliseconds instead of clicking.
            let ramp = frames.min(RAMP_FRAMES);
            let (dl, dr) = ((target.0 - gl) / ramp as f32, (target.1 - gr) / ramp as f32);
            for f in 0..frames {
                if v.cursor >= len as f64 {
                    if v.play.looping {
                        v.cursor %= len as f64;
                    } else {
                        break;
                    }
                }
                let i = v.cursor as usize;
                let t = (v.cursor - i as f64) as f32;
                let a = v.play.buffer.frame(i);
                let next = if i + 1 < len {
                    i + 1
                } else if v.play.looping {
                    0
                } else {
                    i
                };
                let b = v.play.buffer.frame(next);
                if f < ramp {
                    gl += dl;
                    gr += dr;
                }
                out[f * 2] += (a.0 + (b.0 - a.0) * t) * gl;
                out[f * 2 + 1] += (a.1 + (b.1 - a.1) * t) * gr;
                v.cursor += step;
            }
            v.current = target;
        }
        self.voices
            .retain(|v| v.play.looping || v.cursor < v.play.buffer.frames() as f64);
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Sound banks read by name.
#[derive(Debug, Default)]
pub struct SoundLibrary {
    /// Each bank: the name it was asked for (or its block's, `veh_jeep`),
    /// that name's hash, and its sounds.
    banks: Vec<(String, u32, Vec<SoundBuffer>)>,
}

impl SoundLibrary {
    /// Reads every bank in blocks whose path contains one of `names`.
    pub fn load<R: Read + Seek>(
        source: &mut AssetSource<R>,
        names: &[&str],
    ) -> Result<Self, AssetError> {
        Self::load_blocks(source, |path| {
            names
                .iter()
                .find(|n| path.contains(*n))
                .map(|n| ((*n).to_owned(), name_hash(n)))
        })
    }

    /// Reads the banks of the blocks whose name (`veh_jeep` of
    /// `blocks\VZ\veh_jeep_P000_Q3.block`) hashes to one of `hashes`: the
    /// template database names a vehicle's engine bank so.
    pub fn load_hashes<R: Read + Seek>(
        source: &mut AssetSource<R>,
        hashes: &[u32],
    ) -> Result<Self, AssetError> {
        Self::load_blocks(source, |path| {
            let name = crate::world::layer_name(path);
            let hash = name_hash(&name);
            hashes.contains(&hash).then_some((name, hash))
        })
    }

    /// Reads the banks whose own (asset) name hashes to one of `hashes`,
    /// wherever they sit (`fol_shared`, one of the resident block's
    /// several), each under its hash written out (`b796ae64`); banks
    /// missing or unreadable are left out.
    pub fn load_banks<R: Read + Seek>(source: &mut AssetSource<R>, hashes: &[u32]) -> Self {
        let banks = hashes
            .iter()
            .filter_map(|&h| {
                let data = source.asset(h, SOUND_BANK_TYPE).ok()?;
                let bank = SoundBank::parse(&data).ok()?;
                Some((
                    format!("{h:08x}"),
                    h,
                    bank.sounds.iter().map(buffer).collect(),
                ))
            })
            .collect();
        Self { banks }
    }

    /// Reads the banks of the blocks `pick` names (by path), under its name
    /// and hash.
    fn load_blocks<R: Read + Seek>(
        source: &mut AssetSource<R>,
        pick: impl Fn(&str) -> Option<(String, u32)>,
    ) -> Result<Self, AssetError> {
        let index = source.archive().index();
        let mut blocks: Vec<(usize, String, u32)> = index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == SOUND_BANK_TYPE)
            .map(|a| usize::from(a.block))
            .filter_map(|b| {
                let (name, hash) = pick(&index.blocks()[b].path)?;
                Some((b, name, hash))
            })
            .collect();
        blocks.sort_unstable();
        blocks.dedup();
        let mut banks: Vec<(String, u32, Vec<SoundBuffer>)> = Vec::new();
        for (b, name, hash) in blocks {
            let decoded = source.block(b)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash != SOUND_BANK_TYPE {
                    continue;
                }
                let Ok(bank) = SoundBank::parse(data) else {
                    continue;
                };
                let sounds = bank.sounds.iter().map(buffer);
                match banks.iter_mut().find(|(n, _, _)| *n == name) {
                    Some((_, _, list)) => list.extend(sounds),
                    None => banks.push((name.clone(), hash, sounds.collect())),
                }
            }
        }
        Ok(Self { banks })
    }

    /// The sounds of bank `name`.
    pub fn bank(&self, name: &str) -> &[SoundBuffer] {
        self.banks
            .iter()
            .find(|(n, _, _)| n == name)
            .map_or(&[], |(_, _, s)| s.as_slice())
    }

    /// The name and sounds of the bank whose name hashes to `hash`.
    pub fn bank_hash(&self, hash: u32) -> Option<(&str, &[SoundBuffer])> {
        self.banks
            .iter()
            .find(|(_, h, _)| *h == hash)
            .map(|(n, _, s)| (n.as_str(), s.as_slice()))
    }

    /// The loudest short sound in bank `name` no longer than `max_seconds`:
    /// a fair guess at a weapon's shot, since sounds are named only by hash.
    pub fn loudest(&self, name: &str, max_seconds: f32) -> Option<&SoundBuffer> {
        self.bank(name)
            .iter()
            .filter(|s| s.seconds() <= max_seconds && s.frames() > 0)
            .max_by(|a, b| a.loudness().total_cmp(&b.loudness()))
    }

    /// The longest sound in bank `name`: usually its loop (an engine, a
    /// place's ambience).
    pub fn longest(&self, name: &str) -> Option<&SoundBuffer> {
        self.bank(name)
            .iter()
            .max_by(|a, b| a.frames().cmp(&b.frames()))
    }
}

/// Every cue table the world archive holds (`ui_hud`'s, the music's, and
/// whatever others its blocks carry), each block read once; tables that
/// don't parse are left out.
pub fn world_cue_tables<R: Read + Seek>(source: &mut AssetSource<R>) -> Vec<CueTable> {
    let index = source.archive().index();
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == CUE_TABLE_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let mut tables: Vec<CueTable> = Vec::new();
    for b in blocks {
        let Ok(decoded) = source.block(b) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (e, data) in contents.iter() {
            if e.type_hash != CUE_TABLE_TYPE {
                continue;
            }
            if let Ok(t) = CueTable::parse(data)
                && !tables.iter().any(|o| o.hash == t.hash)
            {
                tables.push(t);
            }
        }
    }
    tables
}

/// The sounds a template's `SoundKey` (a name hash, `prop_sheetmetal_sml`)
/// plays, each with its weight, how loud, and how they were found, for
/// the log.
#[derive(Clone, Debug, Default)]
pub struct KeySounds {
    pub sounds: Vec<(SoundBuffer, f32)>,
    pub volume: f32,
    pub how: String,
}

impl KeySounds {
    /// What sound key `key` plays, found as the game finds its interface's
    /// sounds (`ui_hud`): a cue of that name in one of the world's cue
    /// `tables`, whose record names its variants (bank, sound, weight);
    /// else a bank named so, each of its sounds alike (as a cue's variants
    /// would be: which is the break the bank doesn't say); else the banks
    /// of a block named so. `None` when none of these is there: the key
    /// names something not yet read.
    pub fn resolve<R: Read + Seek>(
        source: &mut AssetSource<R>,
        tables: &[CueTable],
        key: u32,
    ) -> Option<Self> {
        for t in tables {
            let Some(record) = t.record_of(key) else {
                continue;
            };
            let mut banks: Vec<u32> = record.variants.iter().map(|v| v.0).collect();
            banks.sort_unstable();
            banks.dedup();
            let library = SoundLibrary::load_banks(source, &banks);
            let sounds: Vec<(SoundBuffer, f32)> = record
                .variants
                .iter()
                .filter_map(|&(bank, i, weight)| {
                    let (_, list) = library.bank_hash(bank)?;
                    Some((list.get(i as usize)?.clone(), weight.max(0.0)))
                })
                .filter(|(s, _)| s.frames() > 0)
                .collect();
            if sounds.is_empty() {
                continue;
            }
            let how = format!(
                "cue of table {:08x}: {} of its {} variants (banks {}), volume {:.2}",
                t.hash,
                sounds.len(),
                record.variants.len(),
                banks
                    .iter()
                    .map(|b| format!("{b:08x}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                record.volume
            );
            let volume = if record.volume > 0.0 {
                record.volume
            } else {
                1.0
            };
            return Some(Self {
                sounds,
                volume,
                how,
            });
        }
        let alike = |list: &[SoundBuffer], how: String| {
            let sounds: Vec<(SoundBuffer, f32)> = list
                .iter()
                .filter(|s| s.frames() > 0)
                .map(|s| (s.clone(), 1.0))
                .collect();
            (!sounds.is_empty()).then(|| Self {
                how: format!(
                    "{how}: {} sounds ({}), each alike",
                    sounds.len(),
                    sounds
                        .iter()
                        .map(|(s, _)| format!("{:08x} {:.2} s", s.hash, s.seconds()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                sounds,
                volume: 1.0,
            })
        };
        let library = SoundLibrary::load_banks(source, &[key]);
        if let Some(found) = library
            .bank_hash(key)
            .and_then(|(_, list)| alike(list, "the bank named so".to_owned()))
        {
            return Some(found);
        }
        let library = SoundLibrary::load_hashes(source, &[key]).ok()?;
        let (name, list) = library.bank_hash(key)?;
        alike(list, format!("the banks of block {name}"))
    }

    /// One of its sounds, `pick` (0 to 1) choosing by weight.
    pub fn pick(&self, pick: f32) -> Option<&SoundBuffer> {
        let total: f32 = self.sounds.iter().map(|s| s.1).sum();
        if total <= 0.0 {
            let i = (pick * self.sounds.len() as f32) as usize;
            return self
                .sounds
                .get(i.min(self.sounds.len().saturating_sub(1)))
                .map(|s| &s.0);
        }
        let mut left = pick.clamp(0.0, 1.0) * total;
        for (s, w) in &self.sounds {
            if left <= *w {
                return Some(s);
            }
            left -= w;
        }
        self.sounds.last().map(|s| &s.0)
    }
}

/// A bank's sound, decoded.
fn buffer(s: &m2_assets::sound::Sound) -> SoundBuffer {
    SoundBuffer {
        hash: s.hash,
        channels: s.channels,
        rate: s.rate,
        samples: s
            .pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect(),
    }
}

/// Steady engine loops run this long, seconds (shorter sounds are horns
/// and gear changes, longer ones sweeps).
const LOOP_SHORTEST: f32 = 1.8;
const LOOP_LONGEST: f32 = 8.5;
/// A steady loop's level varies at most this much, dB: from tenth to
/// tenth of it, and (between its quietest and loudest tenth of 0.1 s
/// windows) within them, where a rev or a gear change would show.
const STEADY_TENTHS: f32 = 3.5;
const STEADY_WINDOWS: f32 = 6.0;
/// Most loops an engine crossfades between.
const ENGINE_LOOPS: usize = 4;
/// Engines heard in a playthrough, for banks that name no loops: the bank,
/// the loop that plays, and how far it is pitched idling and flat out. The
/// Xbox 360 playthrough's AMX-30 runs on `veh_tankmed`'s 13515df7 at 0.84
/// to 0.97 of its own pitch, from idle to 10 m/s (the other steady sounds
/// of that bank, measured as loops, are its tracks' and others' it
/// shares with the other tank banks).
const HEARD: [(&str, u32, (f32, f32)); 1] = [("veh_tankmed", 0x1351_5df7, (0.84, 1.0))];

/// What an engine sound is for: a steady loop (0 idling, 1 slow, 2 at
/// middling revs, 3 fast), or starting or stopping the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Loop(usize),
    Start,
    Stop,
}

/// The endings of an engine's sounds' names, by part: `veh_semi_idle_loop`,
/// `veh_bus_slow_loop_02`, a BMW's `_low_loop` and `_hi_loop`.
const PARTS: &[(&str, Part)] = &[
    ("idle_loop", Part::Loop(0)),
    ("slow_loop", Part::Loop(1)),
    ("slow_loop_01", Part::Loop(1)),
    ("slow_loop_02", Part::Loop(1)),
    ("low_loop", Part::Loop(1)),
    ("med_loop", Part::Loop(2)),
    ("fast_loop", Part::Loop(3)),
    ("hi_loop", Part::Loop(3)),
    ("high_loop", Part::Loop(3)),
    ("start", Part::Start),
    ("start_01", Part::Start),
    ("stop", Part::Stop),
];

/// Banks whose sounds are named after the vehicle recorded, not the bank:
/// the fire engine's in `veh_largedieselnew` (`veh_firetruck_idle_loop`),
/// a BMW 5's, an old Cobra's.
const NAMED_AFTER: &[(&str, &str)] = &[
    ("veh_largedieselnew", "veh_firetruck"),
    ("veh_8cylinderluxury", "veh_bmw5"),
    ("veh_8cylinderold", "veh_cobra"),
];

/// And the sounds of banks named after a vehicle not yet known, by hash.
/// Each is `veh_<vehicle>_` and one of [`PARTS`]' endings: every beginning
/// of up to seven letters was tried before every ending, and in each bank
/// these hashes agree on one beginning (a few endings more were found
/// after it). `veh_largegasold` plays the pickup's `slow_loop_02`, which it
/// holds too. The Cobra's bank names its idle and slow loops after the
/// Cobra but its middling and fast ones (`_med`, `_fast`) after another.
const HASHED_PARTS: &[(u32, Part)] = &[
    // veh_4cylindernew
    (0x806e_54cf, Part::Loop(0)),
    (0xb0b6_46f3, Part::Loop(1)),
    (0xd66f_dc8a, Part::Loop(3)),
    (0xc59a_cfb0, Part::Start),
    (0x12d9_a610, Part::Stop),
    // veh_4cylinderracing
    (0xda04_337e, Part::Loop(0)),
    (0x9c68_99c2, Part::Loop(3)),
    (0x7025_6f71, Part::Start),
    (0xa5e4_b543, Part::Stop),
    // veh_8cylinderold (the Cobra's middling and fast loops)
    (0x2194_c12c, Part::Loop(2)),
    (0xec9d_e3e4, Part::Loop(3)),
    // veh_8cylinderjapanese
    (0x1a26_2d1e, Part::Loop(0)),
    (0xc32a_00a0, Part::Loop(1)),
    (0x0c80_36b8, Part::Loop(2)),
    (0x26ae_5f1c, Part::Loop(3)),
    (0xae95_0291, Part::Start),
    (0xde08_1363, Part::Stop),
    // veh_8cylindertruck (the Ford F-150)
    (0x0de2_278b, Part::Loop(0)),
    (0xff10_6b9c, Part::Loop(1)),
    (0x9640_0153, Part::Loop(2)),
    (0x0487_aa57, Part::Loop(3)),
    (0x49e3_4594, Part::Start),
    (0x73df_8c1c, Part::Stop),
    // veh_largedieselold (the LAV III)
    (0x47b1_b7af, Part::Loop(0)),
    (0x225b_7aff, Part::Loop(2)),
    (0x64f9_32c3, Part::Loop(3)),
    (0xf50d_d090, Part::Start),
    (0xc235_e3f0, Part::Stop),
    // veh_largegascrappy (the HMMWV, the Valiant)
    (0x6ade_f5c7, Part::Loop(0)),
    (0xb651_da77, Part::Loop(2)),
    (0x48b8_3c9b, Part::Loop(3)),
    (0x2c70_e102, Part::Start),
    (0x2fab_d948, Part::Stop),
    // veh_largegasnew
    (0x7eb8_4787, Part::Loop(0)),
    (0x9158_db37, Part::Loop(2)),
    (0x0595_f25b, Part::Loop(3)),
    (0xa9c1_e748, Part::Start),
    (0x3170_b608, Part::Stop),
    // veh_largegasold
    (0x9c59_602d, Part::Loop(0)),
    (0x3729_cd61, Part::Loop(1)),
    (0xeee1_ef9a, Part::Start),
    (0xdbb0_da8e, Part::Stop),
    // veh_tankdiesel: its one loop (`_lp`) and its stop.
    (0x4a4f_9c2f, Part::Loop(0)),
    (0xbf33_f8fb, Part::Stop),
];

/// How far a bank's lone loop is pitched idling and flat out, when its
/// name says it is the idle (a tank diesel's, a hatchback's with no
/// other).
const LONE_LOOP: (f32, f32) = (1.0, 1.8);

/// A vehicle engine's sounds from its bank: steady loops from idle up,
/// and the one-shots of starting and stopping it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineSounds {
    /// Steady loops, idle first, each with its pitch (Hz) where one could
    /// be measured.
    pub loops: Vec<(SoundBuffer, Option<f32>)>,
    pub start: Option<SoundBuffer>,
    pub stop: Option<SoundBuffer>,
    /// Picked by name (`veh_semi_idle_loop`, ...), rather than measured.
    pub named: bool,
    /// A single loop's pitch idling and flat out: as heard in a
    /// playthrough ([`HEARD`]), or a bank's only named loop's
    /// ([`LONE_LOOP`]).
    pub heard: Option<(f32, f32)>,
}

impl EngineSounds {
    /// The engine sounds of `sounds`, bank `bank` (`veh_semi`): by name
    /// where the bank names them (`<bank>_idle_loop`, `_slow_loop`,
    /// `_med_loop`, `_fast_loop`, `_start`, `_stop`, or after the vehicle
    /// recorded: [`NAMED_AFTER`], [`HASHED_PARTS`]), else its steady loops
    /// (no revs, sweeps or gear changes), the lowest pitched first, at
    /// most four spread from lowest to highest. Which unnamed one-shot
    /// starts or stops the engine can't be told by measuring: none.
    pub fn pick(bank: &str, sounds: &[SoundBuffer]) -> Self {
        let after = NAMED_AFTER
            .iter()
            .find(|(b, _)| *b == bank)
            .map_or(bank, |(_, vehicle)| *vehicle);
        let part = |s: &SoundBuffer| {
            PARTS
                .iter()
                .find(|(ending, _)| name_hash(&format!("{after}_{ending}")) == s.hash)
                .map(|(_, p)| *p)
                .or_else(|| {
                    HASHED_PARTS
                        .iter()
                        .find(|(h, _)| *h == s.hash)
                        .map(|(_, p)| *p)
                })
        };
        let parts: Vec<(&SoundBuffer, Part)> = sounds
            .iter()
            .filter(|s| s.frames() > 0)
            .filter_map(|s| Some((s, part(s)?)))
            .collect();
        let find = |want: Part| {
            parts
                .iter()
                .find(|(_, p)| *p == want)
                .map(|(s, _)| (*s).clone())
        };
        let loops: Vec<SoundBuffer> = (0..4).filter_map(|k| find(Part::Loop(k))).collect();
        if !loops.is_empty() {
            let lone = loops.len() == 1;
            return Self {
                loops: loops
                    .into_iter()
                    .map(|s| {
                        let f = s.fundamental();
                        (s, f)
                    })
                    .collect(),
                start: find(Part::Start),
                stop: find(Part::Stop),
                named: true,
                heard: lone.then_some(LONE_LOOP),
            };
        }
        if let Some((s, pitch)) = HEARD.iter().find_map(|&(b, hash, pitch)| {
            let s = sounds
                .iter()
                .find(|s| b == bank && s.hash == hash && s.frames() > 0)?;
            Some((s, pitch))
        }) {
            return Self {
                loops: vec![(s.clone(), s.fundamental())],
                start: None,
                stop: None,
                named: false,
                heard: Some(pitch),
            };
        }
        let mut steady: Vec<(&SoundBuffer, Option<f32>)> = sounds
            .iter()
            .filter(|s| (LOOP_SHORTEST..=LOOP_LONGEST).contains(&s.seconds()) && s.is_steady())
            .map(|s| (s, s.fundamental()))
            .collect();
        // By pitch where every loop has one, else by how busy they sound.
        if steady.iter().all(|(_, f)| f.is_some()) {
            steady.sort_by(|a, b| a.1.unwrap_or(0.0).total_cmp(&b.1.unwrap_or(0.0)));
        } else {
            steady.sort_by(|a, b| a.0.crossing_rate().total_cmp(&b.0.crossing_rate()));
        }
        let n = steady.len();
        let picks: Vec<usize> = if n <= ENGINE_LOOPS {
            (0..n).collect()
        } else {
            let mut p: Vec<usize> = (0..ENGINE_LOOPS)
                .map(|k| (k * (n - 1) + (ENGINE_LOOPS - 1) / 2) / (ENGINE_LOOPS - 1))
                .collect();
            p.dedup();
            p
        };
        Self {
            loops: picks
                .into_iter()
                .map(|i| (steady[i].0.clone(), steady[i].1))
                .collect(),
            start: None,
            stop: None,
            named: false,
            heard: None,
        }
    }
}

/// The streamed music and ambience.
#[derive(Debug, Clone)]
pub struct Streams {
    /// `data/Audios`.
    dir: PathBuf,
    pub music: StreamIndex,
    pub ambience: StreamIndex,
}

impl Streams {
    /// Reads the music and ambience indexes from the install's archive.
    pub fn load<R: Read + Seek>(
        install: &Install,
        source: &mut AssetSource<R>,
    ) -> Result<Self, AssetError> {
        let index = source.archive().index();
        let blocks: Vec<usize> = (0..index.blocks().len())
            .filter(|&b| {
                let p = &index.blocks()[b].path;
                p.contains("music") || p.contains("sound_resident")
            })
            .collect();
        let (mut music, mut ambience) = (None, None);
        for b in blocks {
            let decoded = source.block(b)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash != SOUND_BANK_TYPE {
                    continue;
                }
                if let Ok(idx) = StreamIndex::parse(data) {
                    match idx.file.as_str() {
                        "music.pws" => music = Some(idx),
                        "ambience.pws" => ambience = Some(idx),
                        _ => {}
                    }
                }
            }
        }
        let missing = |what: &str| AssetError::Io {
            path: install.world_archive(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no {what} index in the archive"),
            ),
        };
        Ok(Self {
            dir: install.data.join("Audios"),
            music: music.ok_or_else(|| missing("music"))?,
            ambience: ambience.ok_or_else(|| missing("ambience"))?,
        })
    }

    /// Reads and decodes stream `i` of `index` (one of this set's).
    pub fn decode(&self, index: &StreamIndex, i: usize) -> Result<SoundBuffer, AssetError> {
        decode_stream(&self.dir.join(&index.file), index, i)
    }
}

/// Reads and decodes stream `i` of `index` from the stream file at `path`.
fn decode_stream(
    path: &std::path::Path,
    index: &StreamIndex,
    i: usize,
) -> Result<SoundBuffer, AssetError> {
    let io = |source| AssetError::Io {
        path: path.to_path_buf(),
        source,
    };
    let s = index.streams.get(i).ok_or_else(|| {
        io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no stream {i}"),
        ))
    })?;
    let mut f = File::open(path).map_err(io)?;
    f.seek(SeekFrom::Start(u64::from(s.offset))).map_err(io)?;
    let mut data = vec![0u8; s.encoded_len as usize];
    f.read_exact(&mut data).map_err(io)?;
    Ok(SoundBuffer {
        hash: s.hash,
        channels: s.channels,
        rate: s.rate,
        samples: decode_xbox_adpcm(&data, s.channels).into(),
    })
}

/// The characters' voices: the voice stream (`vo_stream.english.pws`)
/// and the resident voice sets' cue tables, from `English.wad`. A set is
/// keyed by the name hash of its name (`vo_jen`); each line names the
/// stream it plays and is keyed by its subtitle's text id.
#[derive(Debug, Clone)]
pub struct Voices {
    path: PathBuf,
    pub index: StreamIndex,
    pub sets: Vec<CueTable>,
}

impl Voices {
    /// Reads the voice index and the `vo_resident` cue tables from the
    /// install's `English.wad`.
    pub fn load(install: &Install) -> Result<Self, AssetError> {
        let wad = install.data.join("English.wad");
        let io = |source| AssetError::Io {
            path: wad.clone(),
            source,
        };
        let file = File::open(&wad).map_err(io)?;
        let mut source = AssetSource::new(std::io::BufReader::new(file))?;
        let paths: Vec<String> = source
            .archive()
            .index()
            .blocks()
            .iter()
            .map(|b| b.path.clone())
            .collect();
        let (mut index, mut sets) = (None, Vec::new());
        for (b, path) in paths.iter().enumerate() {
            let resident = path.contains("vo_resident");
            if !resident && !path.contains("\\resident_P") {
                continue;
            }
            let decoded = source.block(b)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                match e.type_hash {
                    SOUND_BANK_TYPE => {
                        if let Ok(i) = StreamIndex::parse(data)
                            && i.file.starts_with("vo_stream")
                        {
                            index = Some(i);
                        }
                    }
                    CUE_TABLE_TYPE if resident => {
                        if let Ok(t) = CueTable::parse(data) {
                            sets.push(t);
                        }
                    }
                    _ => {}
                }
            }
        }
        let index = index.ok_or_else(|| {
            io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no voice stream index",
            ))
        })?;
        // On disk the stream carries the language: `vo_stream.english.pws`.
        let file = index.file.replace(".pws", ".english.pws");
        Ok(Self {
            path: install.data.join("Audios").join(file),
            index,
            sets,
        })
    }

    /// The voice set keyed by `hash` (`name_hash("vo_jen")`).
    pub fn set(&self, hash: u32) -> Option<&CueTable> {
        self.sets.iter().find(|t| t.hash == hash)
    }

    /// Reads and decodes voice stream `i`.
    pub fn decode(&self, i: usize) -> Result<SoundBuffer, AssetError> {
        decode_stream(&self.path, &self.index, i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, frames: usize) -> SoundBuffer {
        SoundBuffer {
            hash: 1,
            channels: 1,
            rate,
            samples: (0..frames).map(|_| 16384i16).collect(),
        }
    }

    #[test]
    fn a_key_picks_its_sounds_by_weight() {
        let sound = |hash| SoundBuffer {
            hash,
            ..tone(22050, 10)
        };
        let key = KeySounds {
            sounds: vec![(sound(1), 0.25), (sound(2), 0.75)],
            volume: 1.0,
            how: String::new(),
        };
        assert_eq!(key.pick(0.0).map(|s| s.hash), Some(1));
        assert_eq!(key.pick(0.2).map(|s| s.hash), Some(1));
        assert_eq!(key.pick(0.3).map(|s| s.hash), Some(2));
        assert_eq!(key.pick(1.0).map(|s| s.hash), Some(2));
        // No weights: each alike.
        let alike = KeySounds {
            sounds: vec![(sound(1), 0.0), (sound(2), 0.0)],
            ..key
        };
        assert_eq!(alike.pick(0.1).map(|s| s.hash), Some(1));
        assert_eq!(alike.pick(0.9).map(|s| s.hash), Some(2));
        assert!(KeySounds::default().pick(0.5).is_none());
    }

    #[test]
    fn flat_sounds_play_once_at_their_gain() {
        let mut m = Mixer::new(100);
        m.master = 1.0;
        let id = m.play(Play::flat(tone(100, 100)).gain(0.5));
        let mut out = vec![0.0; 400];
        m.mix(&mut out);
        // After the gain glide: half of half scale on both sides.
        assert!((out[2 * 80] - 0.25).abs() < 1e-3, "{}", out[160]);
        assert_eq!(out[2 * 80], out[2 * 80 + 1]);
        assert_eq!(out[2 * 150], 0.0, "finished");
        assert!(!m.is_playing(id));
    }

    #[test]
    fn a_fade_ducks_its_own_category_alone() {
        let mut m = Mixer::new(100);
        m.master = 1.0;
        m.fade(Category::World, 0.25, 1.0);
        m.play(Play::flat(tone(100, 1000)).looped());
        let mut out = vec![0.0; 2 * 50];
        // Half a second in the world is at 0.625; a second in, at 0.25.
        m.mix(&mut out);
        m.mix(&mut out);
        assert!((out[2 * 49] - 0.5 * 0.25).abs() < 1e-3, "{}", out[98]);
        let mut m = Mixer::new(100);
        m.master = 1.0;
        m.fade(Category::World, 0.25, 0.0);
        m.play(Play::flat(tone(100, 1000)).looped().ui());
        m.mix(&mut out);
        m.mix(&mut out);
        assert!((out[2 * 49] - 0.5).abs() < 1e-3, "{}", out[98]);
    }

    #[test]
    fn a_pitched_category_plays_slower() {
        // Recorded at the mixer's rate: at half pitch each source frame
        // lasts two output frames, so a 10-frame sound takes 20.
        let mut m = Mixer::new(100);
        m.pitch(Category::World, 0.5, 0.0);
        let world = m.play(Play::flat(tone(100, 10)));
        let barks = m.play(Play::flat(tone(100, 10)).category(Category::Chatter));
        let mut out = vec![0.0; 2 * 15];
        m.mix(&mut out);
        assert!(m.is_playing(world));
        assert!(!m.is_playing(barks));
        m.mix(&mut out);
        assert!(!m.is_playing(world));
    }

    #[test]
    fn loops_keep_playing_and_resample() {
        let mut m = Mixer::new(100);
        // Recorded at 50 Hz: each source frame lasts two output frames.
        let id = m.play(Play::flat(tone(50, 10)).looped());
        let mut out = vec![0.0; 200];
        m.mix(&mut out);
        m.mix(&mut out);
        assert!(m.is_playing(id));
        assert!(out.iter().all(|&s| s > 0.0));
        m.stop(id);
        assert_eq!(m.voice_count(), 0);
    }

    #[test]
    fn placed_sounds_fade_and_pan() {
        let mut m = Mixer::new(100);
        m.master = 1.0;
        m.set_listener(Vec3::ZERO, Vec3::Z);
        let right = Play::at(tone(100, 1000), Vec3::new(-10.0, 0.0, 0.0)).looped();
        let near = m.gains(&right);
        assert!(near.1 > near.0, "louder in the right ear: {near:?}");
        let far = m.gains(&Play::at(tone(100, 1000), Vec3::new(-100.0, 0.0, 0.0)));
        assert!(far.1 < near.1 * 0.2);
        let gone = m.gains(&Play::at(tone(100, 1000), Vec3::new(400.0, 0.0, 0.0)));
        assert_eq!(gone, (0.0, 0.0));
        // Facing the other way swaps the ears.
        m.set_listener(Vec3::ZERO, -Vec3::Z);
        let turned = m.gains(&right);
        assert!(turned.0 > turned.1);
    }

    #[test]
    fn falloff_is_full_near_and_zero_far() {
        let f = Falloff::default();
        assert_eq!(f.gain(1.0), 1.0);
        assert!((f.gain(8.0) - 0.5).abs() < 1e-6);
        assert_eq!(f.gain(f.far), 0.0);
    }

    #[test]
    fn too_many_voices_drop_the_quietest() {
        let mut m = Mixer::new(100);
        m.max_voices = 2;
        let quiet = m.play(Play::flat(tone(100, 100)).gain(0.1));
        let loud = m.play(Play::flat(tone(100, 100)));
        let third = m.play(Play::flat(tone(100, 100)));
        assert!(!m.is_playing(quiet));
        assert!(m.is_playing(loud) && m.is_playing(third));
    }

    /// An engine-like sound: a pulse every `1 / hz` seconds, decaying,
    /// with its level swinging by `swing` (0 steady) `revs` times over its
    /// length.
    fn engine(hash: u32, hz: f32, seconds: f32, swing: f32, revs: f32) -> SoundBuffer {
        let rate = 22_050;
        let n = (seconds * rate as f32) as usize;
        let samples = (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                // Each firing alike: a ring that dies away.
                let phase = (t * hz).fract();
                let pulse = (-phase * 6.0).exp() * (phase * 3.0 * TAU).sin();
                let level = 1.0 - swing * 0.5 * (1.0 - (t / seconds * revs * TAU).cos());
                (pulse * level * 12_000.0) as i16
            })
            .collect();
        SoundBuffer {
            hash,
            channels: 1,
            rate,
            samples,
        }
    }

    use std::f32::consts::TAU;

    #[test]
    fn an_engines_beat_is_its_pitch() {
        for hz in [30.0, 55.0, 120.0, 240.0] {
            let f = engine(1, hz, 3.0, 0.0, 0.0).fundamental().unwrap();
            assert!((f - hz).abs() < hz * 0.06, "{hz} Hz heard as {f}");
        }
        let mut seed = 0x9e37_79b9_u32;
        let hiss = SoundBuffer {
            samples: (0..44_100)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed >> 20) as i16 - 2048
                })
                .collect(),
            ..engine(1, 50.0, 0.1, 0.0, 0.0)
        };
        assert_eq!(hiss.fundamental(), None);
    }

    #[test]
    fn revs_and_sweeps_are_not_steady() {
        assert!(engine(1, 50.0, 4.0, 0.0, 0.0).is_steady());
        // Revved three times: steady tenth to tenth, not within them.
        let revving = engine(1, 50.0, 4.0, 0.9, 10.0);
        assert!(!revving.is_steady());
        // One slow swell.
        assert!(!engine(1, 50.0, 4.0, 0.9, 0.5).is_steady());
    }

    #[test]
    fn named_engine_sounds_are_picked_by_name() {
        let h = |s: &str| name_hash(&format!("veh_semi_{s}"));
        let bank = [
            engine(h("fast_loop"), 200.0, 4.0, 0.0, 0.0),
            engine(h("Idle_loop"), 40.0, 3.0, 0.0, 0.0),
            engine(h("accel_01"), 90.0, 5.0, 0.9, 0.5),
            engine(h("MED_loop"), 120.0, 3.0, 0.0, 0.0),
            engine(h("start"), 60.0, 1.6, 0.9, 0.5),
            engine(h("stop"), 60.0, 2.8, 0.9, 0.5),
        ];
        let e = EngineSounds::pick("veh_semi", &bank);
        assert!(e.named);
        let order: Vec<u32> = e.loops.iter().map(|l| l.0.hash).collect();
        assert_eq!(order, [h("idle_loop"), h("med_loop"), h("fast_loop")]);
        assert_eq!(e.start.map(|s| s.hash), Some(h("start")));
        assert_eq!(e.stop.map(|s| s.hash), Some(h("stop")));
    }

    #[test]
    fn engines_named_after_another_vehicle_or_by_hash_are_picked_too() {
        // The fire engine's, in a bank named for its engine.
        let h = |s: &str| name_hash(&format!("veh_firetruck_{s}"));
        let bank = [
            engine(h("med_loop"), 120.0, 3.0, 0.0, 0.0),
            engine(h("idle_loop"), 40.0, 3.0, 0.0, 0.0),
            engine(h("slow_loop"), 90.0, 3.0, 0.0, 0.0),
            engine(h("start"), 60.0, 1.6, 0.9, 0.5),
        ];
        let e = EngineSounds::pick("veh_largedieselnew", &bank);
        let order: Vec<u32> = e.loops.iter().map(|l| l.0.hash).collect();
        assert_eq!(order, [h("idle_loop"), h("slow_loop"), h("med_loop")]);
        assert!(e.named && e.start.is_some() && e.stop.is_none());
        // The pickup's, known by hash: idle, slow, medium, fast, whatever
        // their pitch (measured, its fast loop came second).
        let bank = [
            engine(0x0487_aa57, 79.0, 5.1, 0.0, 0.0),
            engine(0x0de2_278b, 24.0, 4.0, 0.0, 0.0),
            engine(0x9640_0153, 80.0, 4.2, 0.0, 0.0),
            engine(0xff10_6b9c, 100.0, 4.3, 0.0, 0.0),
            engine(0x49e3_4594, 60.0, 2.4, 0.9, 0.5),
        ];
        let e = EngineSounds::pick("veh_8cylindertruck", &bank);
        let order: Vec<u32> = e.loops.iter().map(|l| l.0.hash).collect();
        assert_eq!(order, [0x0de2_278b, 0xff10_6b9c, 0x9640_0153, 0x0487_aa57]);
        assert_eq!(e.start.map(|s| s.hash), Some(0x49e3_4594));
        assert_eq!(e.heard, None);
        // The Cobra's: idle and slow by its name, middling and fast by hash.
        let h = |s: &str| name_hash(&format!("veh_cobra_{s}"));
        let bank = [
            engine(0xec9d_e3e4, 150.0, 3.4, 0.0, 0.0),
            engine(0x2194_c12c, 110.0, 2.7, 0.0, 0.0),
            engine(h("idle_loop"), 40.0, 4.9, 0.0, 0.0),
            engine(h("slow_loop"), 80.0, 3.9, 0.0, 0.0),
        ];
        let e = EngineSounds::pick("veh_8cylinderold", &bank);
        let order: Vec<u32> = e.loops.iter().map(|l| l.0.hash).collect();
        assert_eq!(
            order,
            [h("idle_loop"), h("slow_loop"), 0x2194_c12c, 0xec9d_e3e4]
        );
        // A lone named loop is pitched up from its own note.
        let e = EngineSounds::pick(
            "veh_tankdiesel",
            &[engine(0x4a4f_9c2f, 30.0, 4.0, 0.0, 0.0)],
        );
        assert_eq!(e.heard, Some(LONE_LOOP));
    }

    #[test]
    fn each_hashed_engine_sound_has_one_part() {
        for (i, (h, _)) in HASHED_PARTS.iter().enumerate() {
            assert!(
                HASHED_PARTS[i + 1..].iter().all(|(o, _)| o != h),
                "{h:08x} twice"
            );
        }
    }

    #[test]
    fn unnamed_engines_use_their_steady_loops_lowest_first() {
        let bank = [
            engine(1, 160.0, 4.0, 0.0, 0.0),
            engine(2, 45.0, 3.0, 0.0, 0.0),
            // A rev sweep and a horn: not loops.
            engine(3, 70.0, 5.0, 0.9, 0.5),
            engine(4, 300.0, 0.8, 0.0, 0.0),
            engine(5, 90.0, 5.0, 0.0, 0.0),
        ];
        let e = EngineSounds::pick("veh_jeep", &bank);
        assert!(!e.named && e.start.is_none());
        let order: Vec<u32> = e.loops.iter().map(|l| l.0.hash).collect();
        assert_eq!(order, [2, 5, 1]);
        // More than four: four spread from the lowest to the highest.
        let many: Vec<SoundBuffer> = (0..7)
            .map(|k| engine(k, 30.0 + 40.0 * k as f32, 3.0, 0.0, 0.0))
            .collect();
        let e = EngineSounds::pick("veh_jeep", &many);
        let order: Vec<u32> = e.loops.iter().map(|l| l.0.hash).collect();
        assert_eq!(order, [0, 2, 4, 6]);
    }
}
