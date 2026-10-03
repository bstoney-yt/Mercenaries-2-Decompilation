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
//! named only by hash. The `music` bank and one other use a different,
//! streaming layout and are not handled.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 95 sound banks in `vz.wad` of a
//! legally installed copy. 93 have this layout: every record's lengths agree
//! with its channel count, the packed data fits the section with at most a
//! few hundred bytes of padding, and the samples read as smooth 16-bit audio
//! at the packed offsets. No bytes from the archives are committed.

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
