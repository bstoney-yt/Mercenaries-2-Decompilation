//! Voice cue tables: which streamed voice lines a character can say, and how.
//!
//! `English.wad` keeps one cue table (type hash `9f8bca10`) per voice set in
//! its `vo_resident` block (27 tables: soldiers of each faction, civilians,
//! the playable and support characters) and one per mission. The resident
//! tables point into the voice stream index (`vo_stream.pws`, stored on disk
//! as `vo_stream.english.pws`); the mission tables into their own block's
//! sound bank.
//!
//! ```text
//! 0x00  u32  0x1d
//! 0x04  u32  table hash
//! 0x08  u16  record count n, u16 a second count (usually n)
//! 0x0c  u32  table hash again
//! 0x10  u32  where the records start (0x20)
//! 0x14  u32  where the record offset table starts: n × u32, from 0x10
//! 0x18  u32  where the line info starts (24-byte entries, not used here)
//! 0x1c  u32  where the line info ends
//! ```
//!
//! A record of 64 bytes is one line:
//!
//! ```text
//! 0x00  u32  cue hash
//! 0x04  u32  category (`POSITIONAL` or `RADIO`, below)
//! 0x10  f32  volume
//! 0x14  u32  1 on the positional lines inspected, 0 on radio (unchecked)
//! 0x18  f32  distance at which it starts to fade
//! 0x1c  f32  distance beyond which it is not heard
//! 0x38  u32  stream index: which voice stream to play
//! ```
//!
//! A line's stream is the one variant at 0x34: `u32` bank (the voice
//! index's hash), `u32` stream index, `f32` weight. Longer records are
//! groups: one cue with the same first 0x30 bytes that picks at random among
//! several variants, which start at 0x68 and run to the record's end
//! (12 bytes each, weights summing to 1). A line's length is best taken
//! from the stream it plays.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 68 cue tables in `English.wad` of
//! a legally installed copy. In all of them the records run exactly up to the
//! offset table and the offset table exactly up to the line info, and every
//! resident line's stream index is below the 12,988 streams of the voice
//! index. The category names are inferred from their settings (see
//! `POSITIONAL` and `RADIO`). Which table belongs to which character is
//! named only by hash. No bytes from the archives are committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of cue tables.
pub const CUE_TABLE_TYPE: u32 = 0x9f8b_ca10;
/// Category of lines heard in the world from where the speaker stands
/// (every one is gone by 125 m, most fading from 10 m).
pub const POSITIONAL: u32 = 0xfa0b_8dbc;
/// Category of lines heard over the radio (most carry to 1000 m).
pub const RADIO: u32 = 0xd221_dbe8;

const LINE_RECORD: usize = 64;

/// Errors from parsing a cue table.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VoiceError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("no data section")]
    NoData,
    #[error("not a cue table (first word {0:#x})")]
    Magic(u32),
    #[error("record {index} at {offset:#x} is outside the records")]
    Record { index: usize, offset: usize },
}

impl From<OutOfBounds> for VoiceError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One line a voice set can say.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub cue: u32,
    pub category: u32,
    pub volume: f32,
    /// Fades from `min_distance` and is not heard beyond `max_distance`.
    pub min_distance: f32,
    pub max_distance: f32,
    /// Index into the voice stream index (or, in a mission table, into the
    /// mission's sound bank).
    pub stream: u32,
}

impl Line {
    pub fn is_positional(&self) -> bool {
        self.category == POSITIONAL
    }
}

/// One cue that picks at random among several streams.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub cue: u32,
    pub category: u32,
    pub volume: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    /// What it may play: stream index and weight.
    pub streams: Vec<(u32, f32)>,
}

impl Group {
    pub fn is_positional(&self) -> bool {
        self.category == POSITIONAL
    }
}

/// A parsed cue table.
#[derive(Debug, Clone, PartialEq)]
pub struct CueTable {
    pub hash: u32,
    pub lines: Vec<Line>,
    pub groups: Vec<Group>,
    /// Every record by its index: volume, and what it may play (bank hash,
    /// sound or stream index, weight).
    pub records: Vec<Record>,
    /// The cue list's plain cues. (Cues that mix several records, flagged
    /// 0x100, are not read.)
    pub cues: Vec<PlainCue>,
}

/// A plain cue: its name hash, the record it plays, and how long it
/// lasts (seconds; negative: it loops, as the music's states do).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlainCue {
    pub cue: u32,
    pub record: u32,
    /// A value not yet known (0.5 on most cues).
    pub a: f32,
    pub seconds: f32,
}

/// A record as the cue list points at it.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// The cue hash its first word holds (0 for a record too short to
    /// hold one).
    pub cue: u32,
    pub volume: f32,
    pub variants: Vec<(u32, u32, f32)>,
}

/// A plain cue in the cue list: name hash, flags (0), two values not yet
/// known, the table's hash, the record index.
const PLAIN_CUE: usize = 24;

impl CueTable {
    /// The record cue `cue` (a name hash, `ui_HUD_Objective_New`) plays
    /// through the cue list.
    pub fn cue_record(&self, cue: u32) -> Option<&Record> {
        let c = self.plain_cue(cue)?;
        self.records.get(c.record as usize)
    }

    /// The record cue `cue` plays: through the cue list
    /// ([`cue_record`](Self::cue_record)), else the record whose own first
    /// word is the cue (a line's or a group's).
    pub fn record_of(&self, cue: u32) -> Option<&Record> {
        self.cue_record(cue)
            .or_else(|| self.records.iter().find(|r| r.cue == cue && cue != 0))
    }

    /// The plain cue keyed by `cue`.
    pub fn plain_cue(&self, cue: u32) -> Option<&PlainCue> {
        self.cues.iter().find(|c| c.cue == cue)
    }

    /// The cue keyed by `cue` (a line, or a group's streams), as the
    /// streams it may play.
    pub fn streams(&self, cue: u32) -> Option<Vec<u32>> {
        if let Some(l) = self.lines.iter().find(|l| l.cue == cue) {
            return Some(vec![l.stream]);
        }
        let g = self.groups.iter().find(|g| g.cue == cue)?;
        Some(g.streams.iter().map(|s| s.0).collect())
    }
}

/// Where a group's variants start.
const GROUP_VARIANTS: usize = 0x68;

impl CueTable {
    /// Parses a cue table asset.
    pub fn parse(asset: &[u8]) -> Result<Self, VoiceError> {
        let data = Ucfx::parse(asset)?
            .top_level(*b"data")
            .ok_or(VoiceError::NoData)?;
        Self::parse_data(data)
    }

    /// Parses a cue table's `data` section.
    pub fn parse_data(data: &[u8]) -> Result<Self, VoiceError> {
        let mut r = Reader::at(data, 0);
        let magic = r.u32()?;
        if magic != 0x1d {
            return Err(VoiceError::Magic(magic));
        }
        let hash = r.u32()?;
        let count = usize::from(r.u16()?);
        let _second_count = r.u16()?;
        let _hash_again = r.u32()?;
        let (records, offsets) = (r.u32()? as usize, r.u32()? as usize);
        let (list, list_end) = (r.u32()? as usize, r.u32()? as usize);

        let mut r = Reader::at(data, offsets);
        let starts = (0..count)
            .map(|_| r.u32().map(|o| o as usize))
            .collect::<Result<Vec<_>, _>>()?;
        let (mut lines, mut groups) = (Vec::new(), Vec::new());
        let mut all = Vec::with_capacity(count);
        for (index, &start) in starts.iter().enumerate() {
            // A record runs to the next one, the last to the offset table.
            let end = starts
                .get(index + 1)
                .copied()
                .unwrap_or_else(|| offsets.saturating_sub(records));
            let offset = records.saturating_add(start);
            if end < start || records.saturating_add(end) > offsets {
                return Err(VoiceError::Record { index, offset });
            }
            let size = end - start;
            if size >= LINE_RECORD {
                let rec = data
                    .get(offset..offset + size)
                    .ok_or(VoiceError::Record { index, offset })?;
                let word = |at: usize| {
                    u32::from_le_bytes([rec[at], rec[at + 1], rec[at + 2], rec[at + 3]])
                };
                let first = if size == LINE_RECORD {
                    0x34
                } else {
                    GROUP_VARIANTS
                };
                all.push(Record {
                    cue: word(0),
                    volume: f32::from_bits(word(0x10)),
                    variants: (first..size)
                        .step_by(12)
                        .filter(|at| at + 12 <= size)
                        .map(|at| (word(at), word(at + 4), f32::from_bits(word(at + 8))))
                        .collect(),
                });
            } else {
                all.push(Record {
                    cue: 0,
                    volume: 0.0,
                    variants: Vec::new(),
                });
            }
            if end - start != LINE_RECORD {
                let size = end - start;
                if size < GROUP_VARIANTS {
                    continue;
                }
                let rec = data
                    .get(offset..offset + size)
                    .ok_or(VoiceError::Record { index, offset })?;
                let word = |at: usize| {
                    u32::from_le_bytes([rec[at], rec[at + 1], rec[at + 2], rec[at + 3]])
                };
                groups.push(Group {
                    cue: word(0),
                    category: word(4),
                    volume: f32::from_bits(word(0x10)),
                    min_distance: f32::from_bits(word(0x18)),
                    max_distance: f32::from_bits(word(0x1c)),
                    streams: (GROUP_VARIANTS..size)
                        .step_by(12)
                        .filter(|at| at + 12 <= size)
                        .map(|at| (word(at + 4), f32::from_bits(word(at + 8))))
                        .collect(),
                });
                continue;
            }
            let rec: [u8; LINE_RECORD] = Reader::at(data, offset).array()?;
            let word =
                |at: usize| u32::from_le_bytes([rec[at], rec[at + 1], rec[at + 2], rec[at + 3]]);
            lines.push(Line {
                cue: word(0),
                category: word(4),
                volume: f32::from_bits(word(0x10)),
                min_distance: f32::from_bits(word(0x18)),
                max_distance: f32::from_bits(word(0x1c)),
                stream: word(0x38),
            });
        }
        // Plain cues in the cue list, found where they stand (the list
        // also holds longer cues whose layout is not known, so it cannot
        // be walked entry by entry).
        let mut cues = Vec::new();
        if let Some(region) = data.get(list..list_end.min(data.len())) {
            let word = |at: usize| {
                u32::from_le_bytes([region[at], region[at + 1], region[at + 2], region[at + 3]])
            };
            let mut at = 0;
            while at + PLAIN_CUE <= region.len() {
                let record = word(at + 20);
                if word(at) != 0
                    && word(at + 4) == 0
                    && word(at + 16) == hash
                    && (record as usize) < count
                {
                    cues.push(PlainCue {
                        cue: word(at),
                        record,
                        a: f32::from_bits(word(at + 8)),
                        seconds: f32::from_bits(word(at + 12)),
                    });
                    at += PLAIN_CUE;
                } else {
                    at += 4;
                }
            }
        }
        Ok(Self {
            hash,
            lines,
            groups,
            records: all,
            cues,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table with the given records (64 bytes for a line).
    fn table(records: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = records.concat();
        let offsets_at = 0x20 + body.len();
        let info_at = offsets_at + 4 * records.len();
        let n = records.len() as u32;
        let mut d: Vec<u8> = [
            0x1d,
            0xabcd,
            n | n << 16,
            0xabcd,
            0x20,
            offsets_at as u32,
            info_at as u32,
            info_at as u32,
        ]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
        d.extend(&body);
        let mut at = 0u32;
        for rec in records {
            d.extend(at.to_le_bytes());
            at += rec.len() as u32;
        }
        d
    }

    fn line(cue: u32, category: u32, min: f32, max: f32, stream: u32) -> Vec<u8> {
        let mut w = [0u32; 16];
        w[0] = cue;
        w[1] = category;
        w[4] = 0.7f32.to_bits();
        w[5] = u32::from(category == POSITIONAL);
        w[6] = min.to_bits();
        w[7] = max.to_bits();
        w[14] = stream;
        w.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn group(cue: u32, streams: &[u32]) -> Vec<u8> {
        let mut w = vec![0u32; GROUP_VARIANTS / 4];
        w[0] = cue;
        w[1] = POSITIONAL;
        w[4] = 1.0f32.to_bits();
        for &s in streams {
            w.extend([0x4216_80b7, s, (1.0 / streams.len() as f32).to_bits()]);
        }
        w.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn parses_lines_and_groups() {
        let d = table(&[
            line(1, POSITIONAL, 35.0, 125.0, 609),
            group(9, &[7, 8]),
            line(2, RADIO, 10.0, 1000.0, 610),
        ]);
        let t = CueTable::parse_data(&d).unwrap();
        assert_eq!((t.hash, t.groups.len(), t.lines.len()), (0xabcd, 1, 2));
        assert_eq!(t.groups[0].streams, [(7, 0.5), (8, 0.5)]);
        assert!(t.groups[0].is_positional());
        assert_eq!(t.streams(9), Some(vec![7, 8]));
        assert_eq!(t.streams(2), Some(vec![610]));
        assert_eq!(t.streams(3), None);
        let (a, b) = (&t.lines[0], &t.lines[1]);
        assert!(a.is_positional() && !b.is_positional());
        assert_eq!((a.cue, a.stream), (1, 609));
        assert_eq!(
            (a.min_distance, a.max_distance, a.volume),
            (35.0, 125.0, 0.7)
        );
        assert_eq!((b.cue, b.stream, b.max_distance), (2, 610, 1000.0));
        let asset = crate::testutil::ucfx(&[crate::testutil::UcfxNode::Leaf(*b"data", d)]);
        assert_eq!(CueTable::parse(&asset).unwrap().lines.len(), 2);
    }

    #[test]
    fn finds_plain_cues_in_the_cue_list() {
        let mut d = table(&[
            line(1, RADIO, 10.0, 1000.0, 15),
            line(2, RADIO, 10.0, 1000.0, 16),
        ]);
        let list = d.len() as u32;
        // A longer cue first (its layout unknown), then a plain one.
        let words: [u32; 12] = [
            0x77,
            0x100,
            0,
            0,
            0x2_0000,
            0x200,
            0xabcd,
            0,
            0, // longer cue
            0x55,
            0,
            0x3f00_0000,
        ];
        let tail: [u32; 3] = [0x3e00_0000, 0xabcd, 1];
        d.extend(words.iter().chain(&tail).flat_map(|w| w.to_le_bytes()));
        let end = d.len() as u32;
        d[0x18..0x1c].copy_from_slice(&list.to_le_bytes());
        d[0x1c..0x20].copy_from_slice(&end.to_le_bytes());
        let t = CueTable::parse_data(&d).unwrap();
        assert_eq!(
            t.cues,
            [PlainCue {
                cue: 0x55,
                record: 1,
                a: 0.5,
                seconds: 0.125
            }]
        );
        let r = t.cue_record(0x55).unwrap();
        assert_eq!(
            (r.volume, r.variants.as_slice()),
            (0.7, &[(0, 16, 0.0)][..])
        );
        assert!(t.cue_record(0x77).is_none());
    }

    #[test]
    fn rejects_bad_tables() {
        let mut d = table(&[line(1, POSITIONAL, 1.0, 2.0, 3)]);
        assert!(matches!(
            CueTable::parse_data(&d[..40]),
            Err(VoiceError::Truncated { .. })
        ));
        // A record offset past the records.
        d[0x20 + 64..0x20 + 68].copy_from_slice(&100u32.to_le_bytes());
        assert!(matches!(
            CueTable::parse_data(&d),
            Err(VoiceError::Record { .. })
        ));
        d[0] = 7;
        assert_eq!(CueTable::parse_data(&d), Err(VoiceError::Magic(7)));
    }

    #[test]
    fn byte_flips_never_panic() {
        let d = table(&[line(1, POSITIONAL, 1.0, 2.0, 3), vec![0; 140]]);
        for at in 0..d.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut b = d.clone();
                b[at] = v;
                let _ = CueTable::parse_data(&b);
            }
        }
    }
}
