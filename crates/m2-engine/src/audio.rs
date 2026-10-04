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
use m2_assets::{BlockContents, CueTable, SoundBank, StreamIndex};
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
            max_voices: 64,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
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
        // Left-handed, +y up: right is forward turned clockwise from above.
        let f = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::Z);
        self.right = Vec3::new(f.z, 0.0, -f.x);
    }

    /// Left and right gain for `play` from the listener.
    fn gains(&self, play: &Play) -> (f32, f32) {
        let g = play.gain * self.master;
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
        let gains: Vec<(f32, f32)> = self.voices.iter().map(|v| self.gains(&v.play)).collect();
        for (v, target) in self.voices.iter_mut().zip(gains) {
            let len = v.play.buffer.frames();
            if len == 0 {
                v.cursor = f64::MAX;
                continue;
            }
            let step = f64::from(v.play.buffer.rate) / f64::from(self.rate)
                * f64::from(v.play.pitch.max(0.01));
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
    banks: Vec<(String, Vec<SoundBuffer>)>,
}

impl SoundLibrary {
    /// Reads every bank in blocks whose path contains one of `names`.
    pub fn load<R: Read + Seek>(
        source: &mut AssetSource<R>,
        names: &[&str],
    ) -> Result<Self, AssetError> {
        let index = source.archive().index();
        let mut blocks: Vec<(usize, String)> = index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == SOUND_BANK_TYPE)
            .map(|a| usize::from(a.block))
            .filter_map(|b| {
                let path = &index.blocks()[b].path;
                names
                    .iter()
                    .find(|n| path.contains(*n))
                    .map(|n| (b, (*n).to_owned()))
            })
            .collect();
        blocks.sort_unstable();
        blocks.dedup();
        let mut banks: Vec<(String, Vec<SoundBuffer>)> = Vec::new();
        for (b, name) in blocks {
            let decoded = source.block(b)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash != SOUND_BANK_TYPE {
                    continue;
                }
                let Ok(bank) = SoundBank::parse(data) else {
                    continue;
                };
                let sounds = bank.sounds.iter().map(|s| SoundBuffer {
                    hash: s.hash,
                    channels: s.channels,
                    rate: s.rate,
                    samples: s
                        .pcm
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]]))
                        .collect(),
                });
                match banks.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, list)) => list.extend(sounds),
                    None => banks.push((name.clone(), sounds.collect())),
                }
            }
        }
        Ok(Self { banks })
    }

    /// The sounds of bank `name`.
    pub fn bank(&self, name: &str) -> &[SoundBuffer] {
        self.banks
            .iter()
            .find(|(n, _)| n == name)
            .map_or(&[], |(_, s)| s.as_slice())
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
        let right = Play::at(tone(100, 1000), Vec3::new(10.0, 0.0, 0.0)).looped();
        let near = m.gains(&right);
        assert!(near.1 > near.0, "louder in the right ear: {near:?}");
        let far = m.gains(&Play::at(tone(100, 1000), Vec3::new(100.0, 0.0, 0.0)));
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
}
