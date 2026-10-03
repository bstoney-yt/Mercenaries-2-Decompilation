//! Sound banks: the sound effects and ambience loops.
//!
//! A sound bank (asset type hash `f753f6d0`) is a `UCFX` container whose
//! `data` section holds uncompressed 16-bit PCM:
//!
//! ```text
//! 0x00  u32  version (0x1d)
//! 0x04  u32  the bank's name hash
//! 0x08  u32  sound count n
//! 0x0C  u32  the bank's name hash again
//! 0x10  u32  0x18 (header length)
//! 0x14  u32  0
//! 0x18  n × 36-byte records:
//!       0x00 u32 sound name hash
//!       0x04 u8 0, u8 channels (1 or 2), u8 2 (16-bit), u8 0
//!       0x08 u32 sample rate (Hz)
//!       0x0C u32 data length in bytes
//!       0x10 u32 frames (length / 2 / channels)
//!       0x14 u32 x 3  0
//!       0x20 u32 an offset (not needed: see below)
//! then the sounds' samples, packed in record order, signed 16-bit little
//! endian, channels interleaved, and a little padding.
//! ```
//!
//! Each sound starts where the previous one ends, right after the record
//! table. The record's last word is not a plain offset from any fixed point
//! (it falls 36 bytes further short of the packed position with each
//! record) and is kept as read.
//!
//! The banks sit in blocks named after what uses them (`wpn_assaultrifle`,
//! `veh_jeep`, `veh_helicoptermedium`, `amb_citymed`, `bulding_destruct`,
//! ...); a sound's role within a bank (a shot, a reload, an engine loop) is
//! named only by hash.
//!
//! # Streamed banks
//!
//! Two banks (`music`, and one in `sound_resident` for ambience) are
//! indexes into the big `data/Audios/*.pws` files ([`StreamIndex`]):
//!
//! ```text
//! 0x00  u32  version (0x1d)
//! 0x04  u32  the bank's name hash
//! 0x08  u16  record count n, u16 1 (streamed)
//! 0x0C  u32  the bank's name hash again
//! 0x10  u32  0x28 (header length)
//! 0x14  u32  0
//! 0x18  char[16]  the stream file's name, NUL-padded ("music.pws")
//! 0x28  n × 36-byte records:
//!       0x00 u32 hash, 0x04 u8 0, u8 channels, u8 4 (Xbox IMA ADPCM), u8 0,
//!       0x08 u32 rate, 0x0C u32 encoded length, 0x10 u32 decoded length
//!       (16-bit PCM bytes), 0x14 u32 x 3 0, 0x20 u32 offset in the file
//! ```
//!
//! The streams are Xbox IMA ADPCM ([`decode_xbox_adpcm`]): blocks of 36
//! bytes per channel, each a 4-byte header (i16 predictor, u8 step index,
//! u8 0) and 32 bytes of 4-bit codes (64 samples), the channels' codes
//! interleaved 4 bytes at a time. Encoded / decoded length is 36 / 128
//! for every record, and the offsets run on from one record to the next.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 95 sound banks in `vz.wad` of a
//! legally installed copy. 93 have this layout: every record's lengths agree
//! with its channel count, the packed data fits the section with at most a
//! few hundred bytes of padding, and the samples read as smooth 16-bit audio
//! at the packed offsets. The other two are the streamed indexes: their
//! records' offsets are cumulative encoded lengths, every encoded length is
//! a whole number of blocks, and the decoded audio is smooth. No bytes from
//! the archives or the stream files are committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of sound banks.
pub const SOUND_BANK_TYPE: u32 = 0xf753_f6d0;

const VERSION: u32 = 0x1d;
const HEADER_LEN: usize = 0x18;
const RECORD_LEN: usize = 36;

/// Errors from parsing a sound bank.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SoundError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("no data section")]
    NoData,
    #[error("unsupported bank version {0:#x}")]
    Version(u32),
    #[error("sound {sound}: {what}")]
    Sound { sound: usize, what: &'static str },
    #[error("a streamed bank (an index into a .pws file): see StreamIndex")]
    Streamed,
}

impl From<OutOfBounds> for SoundError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One sound.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound<'a> {
    pub hash: u32,
    pub channels: u16,
    pub rate: u32,
    pub frames: u32,
    /// The record's last word, as read.
    pub unk_0x20: u32,
    /// Signed 16-bit little-endian samples, channels interleaved.
    pub pcm: &'a [u8],
}

impl Sound<'_> {
    /// Length in seconds.
    pub fn seconds(&self) -> f32 {
        self.frames as f32 / self.rate.max(1) as f32
    }

    /// The sound as a WAV file (RIFF, 16-bit PCM).
    pub fn wav(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(44 + self.pcm.len());
        let block = 2 * u32::from(self.channels);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(self.pcm.len() as u32).saturating_add(36).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&self.rate.saturating_mul(block).to_le_bytes());
        out.extend_from_slice(&(block as u16).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(self.pcm.len() as u32).to_le_bytes());
        out.extend_from_slice(self.pcm);
        out
    }
}

/// A parsed sound bank.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundBank<'a> {
    pub hash: u32,
    pub sounds: Vec<Sound<'a>>,
}

impl<'a> SoundBank<'a> {
    /// Parses a sound bank asset.
    pub fn parse(asset: &'a [u8]) -> Result<Self, SoundError> {
        let data = Ucfx::parse(asset)?
            .top_level(*b"data")
            .ok_or(SoundError::NoData)?;
        Self::parse_data(data)
    }

    /// Parses the bank's `data` section.
    pub fn parse_data(data: &'a [u8]) -> Result<Self, SoundError> {
        let mut r = Reader::at(data, 0);
        let (version, hash, count) = (r.u32()?, r.u32()?, r.u32()?);
        if version != VERSION {
            return Err(SoundError::Version(version));
        }
        if count >> 16 == 1 {
            return Err(SoundError::Streamed);
        }
        let count = count as usize;
        let table_end = count
            .checked_mul(RECORD_LEN)
            .and_then(|n| n.checked_add(HEADER_LEN))
            .ok_or(SoundError::Truncated {
                offset: HEADER_LEN,
                len: usize::MAX,
            })?;
        Reader::at(data, HEADER_LEN).take(table_end - HEADER_LEN)?;
        let mut at = table_end;
        let mut sounds = Vec::with_capacity(count);
        for i in 0..count {
            let mut r = Reader::at(data, HEADER_LEN + i * RECORD_LEN);
            let hash = r.u32()?;
            let format: [u8; 4] = r.array()?;
            let (rate, len, frames) = (r.u32()?, r.u32()?, r.u32()?);
            let _zeros = (r.u32()?, r.u32()?, r.u32()?);
            let unk_0x20 = r.u32()?;
            let bad = |what| SoundError::Sound { sound: i, what };
            let channels = u16::from(format[1]);
            if format[2] != 2 || !(1..=2).contains(&channels) {
                return Err(bad("not 16-bit mono or stereo PCM"));
            }
            if u64::from(frames) * 2 * u64::from(channels) != u64::from(len) {
                return Err(bad("length and frames disagree"));
            }
            let pcm = Reader::at(data, at).take(len as usize)?;
            at += len as usize;
            sounds.push(Sound {
                hash,
                channels,
                rate,
                frames,
                unk_0x20,
                pcm,
            });
        }
        Ok(Self { hash, sounds })
    }
}

/// One stream in a `.pws` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    pub hash: u32,
    pub channels: u16,
    pub rate: u32,
    /// Where the encoded stream is in the `.pws` file, and its length.
    pub offset: u32,
    pub encoded_len: u32,
    /// Length of the decoded 16-bit PCM, in bytes.
    pub decoded_len: u32,
}

impl Stream {
    pub fn seconds(&self) -> f32 {
        self.decoded_len as f32 / 2.0 / f32::from(self.channels.max(1)) / self.rate.max(1) as f32
    }
}

/// A streamed bank: an index into one of the `data/Audios/*.pws` files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamIndex {
    pub hash: u32,
    /// The stream file's name, such as `music.pws`.
    pub file: String,
    pub streams: Vec<Stream>,
}

const STREAM_HEADER_LEN: usize = 0x28;
/// Xbox IMA ADPCM block length per channel.
const ADPCM_BLOCK: usize = 36;

impl StreamIndex {
    /// Parses a streamed bank asset.
    pub fn parse(asset: &[u8]) -> Result<Self, SoundError> {
        let data = Ucfx::parse(asset)?
            .top_level(*b"data")
            .ok_or(SoundError::NoData)?;
        let mut r = Reader::at(data, 0);
        let (version, hash, count) = (r.u32()?, r.u32()?, r.u32()?);
        if version != VERSION {
            return Err(SoundError::Version(version));
        }
        if count >> 16 != 1 {
            return Err(SoundError::Sound {
                sound: 0,
                what: "not a streamed bank",
            });
        }
        let count = (count & 0xffff) as usize;
        let name = Reader::at(data, 0x18).take(16)?;
        let file =
            String::from_utf8_lossy(&name[..name.iter().position(|&b| b == 0).unwrap_or(16)])
                .into_owned();
        let mut streams = Vec::with_capacity(count);
        for i in 0..count {
            let mut r = Reader::at(data, STREAM_HEADER_LEN + i * RECORD_LEN);
            let hash = r.u32()?;
            let format: [u8; 4] = r.array()?;
            let (rate, encoded_len, decoded_len) = (r.u32()?, r.u32()?, r.u32()?);
            let _zeros = (r.u32()?, r.u32()?, r.u32()?);
            let offset = r.u32()?;
            let channels = u16::from(format[1]);
            let bad = |what| SoundError::Sound { sound: i, what };
            if format[2] != 4 || !(1..=2).contains(&channels) {
                return Err(bad("not Xbox IMA ADPCM, mono or stereo"));
            }
            if !(encoded_len as usize).is_multiple_of(ADPCM_BLOCK * usize::from(channels)) {
                return Err(bad("not a whole number of ADPCM blocks"));
            }
            streams.push(Stream {
                hash,
                channels,
                rate,
                offset,
                encoded_len,
                decoded_len,
            });
        }
        Ok(Self {
            hash,
            file,
            streams,
        })
    }
}

const IMA_STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const IMA_INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// Decodes Xbox IMA ADPCM to 16-bit samples, channels interleaved: 64
/// frames per block of 36 bytes per channel. A trailing partial block is
/// ignored.
pub fn decode_xbox_adpcm(data: &[u8], channels: u16) -> Vec<i16> {
    let ch = usize::from(channels.clamp(1, 2));
    let block = ADPCM_BLOCK * ch;
    let mut out = Vec::with_capacity(data.len() / block * 64 * ch);
    let mut frame = vec![[0i16; 64]; ch];
    for b in data.chunks_exact(block) {
        for (c, samples) in frame.iter_mut().enumerate() {
            let h = &b[4 * c..4 * c + 4];
            let mut pred = i32::from(i16::from_le_bytes([h[0], h[1]]));
            let mut index = i32::from(h[2]).min(88);
            // Codes: 8 groups of 4 bytes per channel, interleaved.
            for g in 0..8 {
                let at = 4 * ch + 4 * (g * ch + c);
                for (k, &byte) in b[at..at + 4].iter().enumerate() {
                    for (n, code) in [byte & 0x0f, byte >> 4].into_iter().enumerate() {
                        let step = IMA_STEPS[index as usize];
                        let mut diff = step >> 3;
                        if code & 4 != 0 {
                            diff += step;
                        }
                        if code & 2 != 0 {
                            diff += step >> 1;
                        }
                        if code & 1 != 0 {
                            diff += step >> 2;
                        }
                        pred = if code & 8 != 0 {
                            pred - diff
                        } else {
                            pred + diff
                        }
                        .clamp(-32768, 32767);
                        index = (index + IMA_INDEX[usize::from(code)]).clamp(0, 88);
                        samples[g * 8 + k * 2 + n] = pred as i16;
                    }
                }
            }
        }
        for i in 0..64 {
            for samples in &frame {
                out.push(samples[i]);
            }
        }
    }
    out
}

/// 16-bit samples, channels interleaved, as a WAV file.
pub fn wav_from_samples(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let pcm: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    Sound {
        hash: 0,
        channels,
        rate,
        frames: (samples.len() / usize::from(channels.max(1))) as u32,
        unk_0x20: 0,
        pcm: &pcm,
    }
    .wav()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// A bank with a mono and a stereo sound.
    fn bank() -> Vec<u8> {
        let mut d = le(&[VERSION, 0xabcd, 2, 0xabcd, 0x18, 0]);
        d.extend(le(&[0x1111, 0x0002_0100, 44100, 8, 4, 0, 0, 0, 0x99]));
        d.extend(le(&[0x2222, 0x0002_0200, 22050, 8, 2, 0, 0, 0, 0x88]));
        d.extend([1, 0, 2, 0, 3, 0, 4, 0]);
        d.extend([0xff, 0x7f, 0x00, 0x80, 5, 0, 6, 0]);
        d.extend([0; 6]);
        d
    }

    #[test]
    fn parses_packed_sounds() {
        let d = bank();
        let b = SoundBank::parse_data(&d).unwrap();
        assert_eq!(b.hash, 0xabcd);
        assert_eq!(b.sounds.len(), 2);
        assert_eq!((b.sounds[0].channels, b.sounds[0].frames), (1, 4));
        assert_eq!(b.sounds[0].pcm, &[1, 0, 2, 0, 3, 0, 4, 0]);
        assert_eq!(b.sounds[1].channels, 2);
        assert_eq!(b.sounds[1].pcm[..4], [0xff, 0x7f, 0x00, 0x80]);
        assert!((b.sounds[1].seconds() - 2.0 / 22050.0).abs() < 1e-9);
        let wav = b.sounds[1].wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 2);
        assert_eq!(wav.len(), 44 + 8);
    }

    #[test]
    fn rejects_inconsistent_banks() {
        let mut d = bank();
        d[0] = 0x1e;
        assert_eq!(
            SoundBank::parse_data(&d).unwrap_err(),
            SoundError::Version(0x1e)
        );
        let mut d = bank();
        d[0x18 + 16] = 5; // frames of sound 0
        assert!(matches!(
            SoundBank::parse_data(&d),
            Err(SoundError::Sound { sound: 0, .. })
        ));
        let d = bank();
        assert!(matches!(
            SoundBank::parse_data(&d[..d.len() - 12]),
            Err(SoundError::Truncated { .. })
        ));
    }

    #[test]
    fn decodes_xbox_adpcm() {
        // Mono: predictor 1000, step index 0, all codes 0: a constant.
        let mut b = vec![0u8; 36];
        b[..2].copy_from_slice(&1000i16.to_le_bytes());
        let s = decode_xbox_adpcm(&b, 1);
        assert_eq!(s.len(), 64);
        assert!(s.iter().all(|&x| x == 1000));
        // Code 7 rises, the step growing.
        let mut b = vec![0x77u8; 36];
        b[..4].copy_from_slice(&[0, 0, 0, 0]);
        let s = decode_xbox_adpcm(&b, 1);
        assert_eq!(s[0], 7 + 7 / 2 + 7 / 4); // 7 >> 3 + 7 + 7 >> 1 + 7 >> 2
        // ...until it clips at the top.
        assert!(s.windows(2).all(|w| w[1] >= w[0]) && s[63] == i16::MAX);
        // Stereo: channels interleave, each from its own header.
        let mut b = vec![0u8; 72];
        b[..2].copy_from_slice(&100i16.to_le_bytes());
        b[4..6].copy_from_slice(&(-100i16).to_le_bytes());
        let s = decode_xbox_adpcm(&b, 2);
        assert_eq!(s.len(), 128);
        assert_eq!(&s[..4], &[100, -100, 100, -100]);
        // A partial block is ignored.
        assert!(decode_xbox_adpcm(&b[..50], 2).is_empty());
    }

    #[test]
    fn parses_stream_index() {
        let mut d = le(&[VERSION, 0x4111, 0x0001_0002, 0x4111, 0x28, 0]);
        let mut name = b"music.pws".to_vec();
        name.resize(16, 0);
        d.extend(name);
        d.extend(le(&[0xa, 0x0004_0200, 48000, 72 * 3, 256 * 3, 0, 0, 0, 0]));
        d.extend(le(&[0xb, 0x0004_0100, 44100, 36, 128, 0, 0, 0, 216]));
        let asset = crate::testutil::ucfx(&[crate::testutil::UcfxNode::Leaf(*b"data", d.clone())]);
        let idx = StreamIndex::parse(&asset).unwrap();
        assert_eq!(idx.file, "music.pws");
        assert_eq!(idx.streams.len(), 2);
        assert_eq!((idx.streams[1].offset, idx.streams[1].channels), (216, 1));
        assert!((idx.streams[0].seconds() - 192.0 / 48000.0).abs() < 1e-6);
        // Not a whole number of blocks.
        d[0x28 + 12] = 70;
        let asset = crate::testutil::ucfx(&[crate::testutil::UcfxNode::Leaf(*b"data", d)]);
        assert!(matches!(
            StreamIndex::parse(&asset),
            Err(SoundError::Sound { sound: 0, .. })
        ));
        // A plain bank says it is not one.
        assert_eq!(SoundBank::parse(&asset).unwrap_err(), SoundError::Streamed);
    }

    #[test]
    fn byte_flips_never_panic() {
        let d = bank();
        for at in 0..d.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut b = d.clone();
                b[at] = v;
                if let Ok(bank) = SoundBank::parse_data(&b) {
                    for s in &bank.sounds {
                        let _ = (s.wav(), s.seconds());
                    }
                }
            }
        }
    }
}
