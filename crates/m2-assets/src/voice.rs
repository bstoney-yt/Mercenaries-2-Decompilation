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
//! 0x0c  u8   0: a short record, its level and pitch at 0x2c and 0x30;
//!            1: a long one, their ranges at 0x50 and 0x5c
//! 0x10  f32  priority (not its level: see [`record_level`])
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
    /// The cue list's plain cues.
    pub cues: Vec<PlainCue>,
    /// The cue list's cues that mix several records ([`MixedCue`]).
    pub mixes: Vec<MixedCue>,
}

/// A cue that plays several records at once, each from its own start
/// (flagged 0x100 in the cue list: a gun's shot, `wpn_assaultrifle_fire`,
/// layers six of its bank's records, one 0.57 s in).
///
/// ```text
/// 0x00  u32  cue hash
/// 0x04  u32  flags: 0x100 set (0x10100, 0x140100 on some; the low byte
///            and the top byte clear)
/// 0x08  f32  a value not yet known (0.4 to 0.75 on the guns' shots)
/// 0x0c  f32  how long it lasts, seconds; -1: it loops (the minigun's
///            `wpn_minigun_fire_loop`)
/// ...        a header not decoded, then per track a header (its first
///            word 0x00010000, or 0x00010100 when a block of eight words
///            follows it) and an entry:
///       u32  1, f32 its start (seconds), u32 0x10, u32 table hash,
///       u32 record index, f32 weight
/// ```
///
/// A track's following block, on the pistol's shot, is four zeros, a
/// value (0.179 and 0.359 on its last two tracks), 1.0 and two zeros; on
/// the covert pistol's, every track has one (0.237 to 0.262). It is read
/// as the track's level ([`Track::level`]): as a delay it would hold the
/// silenced pistol's every layer back a quarter second, and summed so
/// its shot peaks at 1.50 of full scale, as a level at 0.44. (The
/// minigun loop's block differs: it names another cue, and is not
/// read.)
///
/// Format-from-samples: tracks are found by that pattern up to the next
/// cue, as the list's layout between them is not known. That the block's
/// value is a level is inferred (round 17 took it for a delay); not
/// checked against the game's code.
#[derive(Debug, Clone, PartialEq)]
pub struct MixedCue {
    pub cue: u32,
    /// How long it lasts, seconds; negative when it loops.
    pub seconds: f32,
    pub tracks: Vec<Track>,
    /// What moves each track while it plays, by track.
    pub curves: Vec<Vec<Curve>>,
}

/// What a [`Curve`] moves.
pub const CURVE_VOLUME: u32 = 5;
pub const CURVE_PITCH: u32 = 6;

/// A track's level or pitch as a parameter the game feeds the cue moves
/// (the vehicles' engines: `15ba509e` their speed, 0 to 1).
///
/// ```text
/// u32  what it moves: 5 volume, 6 pitch
/// u32  0
/// u32  n, its points
/// u32  the parameter's hash
/// u32  0x14, u32 0
/// f32  the value at parameter 0, then n - 1 pairs (parameter, value)
/// ```
///
/// Format-from-samples: over the engine cues every curve's parameters
/// rise to 1. Volumes run 0 to 1; pitches about -10 to 7, read as
/// semitones (inferred from their range).
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub kind: u32,
    pub param: u32,
    /// (parameter, value), rising in the parameter.
    pub points: Vec<(f32, f32)>,
}

impl Curve {
    /// Its value at `x`: joined straight between points, flat past the
    /// ends (inferred).
    pub fn at(&self, x: f32) -> f32 {
        let Some(&first) = self.points.first() else {
            return 0.0;
        };
        let mut last = first;
        for &p in &self.points {
            if x <= p.0 {
                let span = p.0 - last.0;
                return if span > 0.0 {
                    last.1 + (p.1 - last.1) * ((x - last.0) / span).max(0.0)
                } else {
                    p.1
                };
            }
            last = p;
        }
        last.1
    }
}

/// One record a [`MixedCue`] plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    /// The cue table holding the record, and its index there.
    pub table: u32,
    pub record: u32,
    /// Seconds from the cue's start.
    pub start: f32,
    pub weight: f32,
    /// What its record's level is scaled by: its block's value where it
    /// has one (see [`MixedCue`]), else 1.
    pub level: f32,
    /// How its track loops ([`TrackLoop`]).
    pub looping: TrackLoop,
}

/// How a mixed cue's track loops: each track keeps its own time, and
/// when that reaches `end` with passes left it goes back to `start` and
/// the track's records are started again as their own starts come.
///
/// ```text
/// 0x00  u8   passes after the first: 0 none, 0xff for ever
/// 0x01  u8   the track's events (levels, curves)
/// 0x02  u8   the track's records
/// 0x04  f32  where a pass goes back to, seconds (-1 on a track that
///            does not loop)
/// 0x08  f32  where a pass ends, seconds
/// 0x0c  u32  0x1c, then offsets within the track
/// ```
///
/// From the 360 code: a track's update (`0x82842088`) adds the frame's
/// time to its own (+0xb8); when that reaches the header's float at +8
/// and its count of passes (+0xbd, the header's first byte, `0x82841FD8`)
/// is not 0, the records are started again (`0x82841DA8`, `0x82844BE8`),
/// the time becomes the float at +4 plus what it overshot by, and the
/// count goes down by one unless it is 0xff. A record starts once the
/// track's time reaches its start (`0x82844BE8`).
///
/// From samples: `amb_jungle`'s one-shot tracks loop for ever over 0 to
/// 23.2 s, 25.67 to 65 s and 3.4 to 17.2 s, with their records at 5.11,
/// 26.97 and 6.2 s (the `mixhead` example).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackLoop {
    /// Passes after the first; 0xff: for ever.
    pub passes: u8,
    pub start: f32,
    pub end: f32,
}

impl TrackLoop {
    /// A track that plays through once.
    pub const ONCE: Self = Self {
        passes: 0,
        start: -1.0,
        end: -1.0,
    };

    /// Whether it loops for ever.
    pub fn is_endless(&self) -> bool {
        self.passes == 0xff
    }
}

/// A plain cue: its name hash, the record it plays, and how long it
/// lasts (seconds; negative: it loops, as the music's states do).
///
/// The record is the low half of the cue's last word; the high half is
/// not known (0 on most cues; 0x3e99 on the vehicles' engine starts and
/// stops). Format-from-samples: with the low half taken, 64 more cues of
/// the world's tables read (the vehicles' starts and stops), and each
/// plays a sound as long as the cue says, as 211 of 212 other plain cues
/// do (`plaincuelen` example).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlainCue {
    pub cue: u32,
    pub record: u32,
    /// A value not yet known (0.5 on most cues).
    pub a: f32,
    pub seconds: f32,
}

/// The level a record's sound plays at, and its priority (the float at
/// 0x10).
///
/// From the 360 code: a sound starts (`0x82838320`) at the level
/// `0x82842E40` answers: for a record whose byte at 0xc is 0, the float
/// at 0x2c; for one with it set, a level drawn evenly between the
/// floats at 0x50 and 0x54. The float at 0x10, which was taken for the
/// volume here, is the record's priority: `PalSoundInstance::
/// GetWavePriority` (`0x828379C0`) answers it times the sound's fade
/// with distance, for choosing which sounds to drop
/// (`GetLowestPrioritySound`, `0x82837D90`); nothing multiplies it into
/// the level.
///
/// From the data (the `recordvol` example, 1,776 records): every
/// 64-byte record has the byte clear and every longer one has it set;
/// the levels are steps of decibels (0.3162, 0.3981, 0.5012, 0.6310,
/// 0.7079, 1.0), where the float at 0x10 is 0.85 or 0.96 on nearly all.
///
/// The level answered for a range is its middle (the draw itself, and
/// the pitch beside it at 0x30 or 0x5c..0x60, are not played yet).
pub fn record_level(rec: &[u8]) -> (f32, f32) {
    let f = |at: usize| {
        rec.get(at..at + 4)
            .map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let ranged = rec.get(0xc).is_some_and(|&b| b != 0) && rec.len() >= 0x58;
    let level = if ranged {
        (f(0x50) + f(0x54)) * 0.5
    } else {
        f(0x2c)
    };
    (level, f(0x10))
}

/// What a record's sound starts at: a level and a pitch, each drawn
/// evenly between two ends.
///
/// From the 360 code: as a sound starts (`0x82838320`) its pitch is
/// drawn first (`0x82842DA8`: the float at 0x30 of a record whose byte
/// at 0xc is 0, else between the floats at 0x5c and 0x60), then its
/// level (`0x82842E40`, [`record_level`]). The pitch is added to the
/// category's and sent to the voice as `pitch / 24 * 8192`
/// (`0x82838558`: two doubles, 1/24 at `0x82076460` and 8192 at
/// `0x82076458`), so 4096/12 of the voice's units to one of the
/// record's.
///
/// The unit is taken as semitones. That rests on the voice counting
/// 4096 to the octave, which is how the console's audio library states
/// a source voice's pitch (a 4.12 fixed-point count of octaves); the
/// call that hands the number to the library was not traced, so this
/// part is inferred, not read. One thing does not fit and is not
/// explained: the engine turns a category's pitch into the same unit
/// with `6 * log2` (`0x82837598`, 2.0 at `0x820F8D88`, 6.0 at
/// `0x82003CF8`), which would be six to the octave if its input is a
/// plain rate.
///
/// From the data (the `recordvol` example): most records' ends are the
/// same; some levels run 0.631 to 0.794 or 0.038 to 0.094, some pitches
/// -1 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartDraw {
    /// The level's ends.
    pub level: (f32, f32),
    /// The pitch's ends, semitones.
    pub pitch: (f32, f32),
}

impl StartDraw {
    /// A sound at its own level and pitch.
    pub const FLAT: Self = Self {
        level: (1.0, 1.0),
        pitch: (0.0, 0.0),
    };

    /// A record's.
    pub fn of(rec: &[u8]) -> Self {
        let f = |at: usize| {
            rec.get(at..at + 4)
                .map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        if rec.get(0xc).is_some_and(|&b| b != 0) && rec.len() >= 0x64 {
            Self {
                level: (f(0x50), f(0x54)),
                pitch: (f(0x5c), f(0x60)),
            }
        } else {
            Self {
                level: (f(0x2c), f(0x2c)),
                pitch: (f(0x30), f(0x30)),
            }
        }
    }

    /// The level a draw of 0 to 1 gives.
    pub fn level(&self, draw: f32) -> f32 {
        self.level.0 + (self.level.1 - self.level.0) * draw.clamp(0.0, 1.0)
    }

    /// What a draw of 0 to 1 scales the middle level by (1 for a level
    /// with no spread): for a sound played at its record's middle level.
    pub fn level_scale(&self, draw: f32) -> f32 {
        let middle = (self.level.0 + self.level.1) * 0.5;
        if middle > 0.0 && self.level.0 != self.level.1 {
            self.level(draw) / middle
        } else {
            1.0
        }
    }

    /// The pitch a draw of 0 to 1 gives, as a rate (1: its own).
    pub fn rate(&self, draw: f32) -> f32 {
        let semitones = self.pitch.0 + (self.pitch.1 - self.pitch.0) * draw.clamp(0.0, 1.0);
        if semitones.is_finite() {
            2.0_f32.powf(semitones / 12.0)
        } else {
            1.0
        }
    }
}

/// A record as the cue list points at it.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// The cue hash its first word holds (0 for a record too short to
    /// hold one).
    pub cue: u32,
    /// The level it plays at ([`record_level`]).
    pub volume: f32,
    /// Fades from `min_distance` and is not heard beyond `max_distance`
    /// (metres: the guns' shots 10 and 1000 or so), as a line's (0x18 and
    /// 0x1c); 0 for a record too short to hold them.
    pub min_distance: f32,
    pub max_distance: f32,
    /// The level and pitch its sound starts at.
    pub start: StartDraw,
    /// Whether it is placed in the world and fades with distance (the
    /// byte at 0x14), and the power its fade follows (the float at 0x24).
    ///
    /// From the 360 code: the sound engine's distance check
    /// (`0x828375F8`) skips a record whose distance block's first byte
    /// is 0, and the fade (`0x828424A0`) reads the block's floats at
    /// +4, +8 and +0x10 as the near distance, the far distance and the
    /// power. The block is taken to start at the record's 0x14, where
    /// its near and far land on the distances already read (0x18 and
    /// 0x1c): inferred from that fit, with the powers the data holds
    /// there (see the `recorddist` example).
    pub positional: bool,
    pub exponent: f32,
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

    /// The mixed cue keyed by `cue`.
    pub fn mix(&self, cue: u32) -> Option<&MixedCue> {
        self.mixes.iter().find(|m| m.cue == cue)
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
                    volume: record_level(rec).0,
                    min_distance: f32::from_bits(word(0x18)),
                    max_distance: f32::from_bits(word(0x1c)),
                    start: StartDraw::of(rec),
                    positional: rec[0x14] != 0,
                    exponent: f32::from_bits(word(0x24)),
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
                    min_distance: 0.0,
                    max_distance: 0.0,
                    start: StartDraw::FLAT,
                    positional: false,
                    exponent: 0.0,
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
                    volume: record_level(rec).0,
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
                volume: record_level(&rec).0,
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
                // The record index is the word's low half; the high half
                // is not known (see [`PlainCue`]).
                let record = word(at + 20) & 0xffff;
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
        let mixes = data
            .get(list..list_end.min(data.len()))
            .map_or_else(Vec::new, |region| mixed_cues(region, hash, count));
        Ok(Self {
            hash,
            lines,
            groups,
            records: all,
            cues,
            mixes,
        })
    }
}

/// A mixed cue's track header's first word (0x100 set when a block of
/// eight words follows it; see [`MixedCue`]).
const TRACK_HEADER: u32 = 0x0001_0000;

/// The mixed cues of a cue list `region` of table `hash` with `count`
/// records: each cue flagged 0x100 with the tracks found after it, up to
/// the next cue, plain or mixed (see [`MixedCue`]).
fn mixed_cues(region: &[u8], hash: u32, count: usize) -> Vec<MixedCue> {
    let word = |at: usize| {
        region
            .get(at..at + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let float = |at: usize| f32::from_bits(word(at));
    let mut mixes = Vec::new();
    let mut current: Option<MixedCue> = None;
    let mut pending: Vec<Curve> = Vec::new();
    let mut looping = TrackLoop::ONCE;
    // A loop's time: -1 for none, else seconds.
    let time = |t: f32| t == -1.0 || (0.0..36000.0).contains(&t);
    let close = |m: Option<MixedCue>, mixes: &mut Vec<MixedCue>| {
        if let Some(m) = m.filter(|m| !m.tracks.is_empty()) {
            mixes.push(m);
        }
    };
    let mut at = 0;
    while at + 4 <= region.len() {
        let (w, flags) = (word(at), word(at + 4));
        let (a, seconds) = (float(at + 8), float(at + 12));
        // A track's header ([`TrackLoop`]): its counts, its loop and the
        // 0x1c its offsets start with.
        if current.is_some()
            && at + 20 <= region.len()
            && w >> 24 == 0
            && (1..=16).contains(&(w >> 16 & 0xff))
            && w >> 8 & 0xff <= 16
            && word(at + 12) == 0x1c
            && (0x1c..0x1000).contains(&word(at + 16))
            && time(float(at + 4))
            && time(float(at + 8))
        {
            looping = TrackLoop {
                passes: w as u8,
                start: float(at + 4),
                end: float(at + 8),
            };
            at += 16;
            continue;
        }
        // A cue's hash, not a small offset (a track header's 0x44 is
        // followed by 0x10100, 0.2 and 0.7 on the minigun's loop).
        // Nor a track header (0x00010000 or 0x00010100), which a cue's
        // own header block may hold followed by 0x100, 1.0 and -1 (the
        // Semi's engine start): taken for a cue, it would end the cue it
        // stands in.
        if at + 16 <= region.len()
            && w > 0xffff
            && w != hash
            && w != TRACK_HEADER
            && w != TRACK_HEADER | 0x100
            && flags & 0xff00 == 0x100
            && flags & 0xff == 0
            && flags >> 24 == 0
            && a > 0.0
            && a <= 2.0
            && ((seconds > 0.0 && seconds < 600.0) || seconds == -1.0)
        {
            close(current.take(), &mut mixes);
            current = Some(MixedCue {
                cue: w,
                seconds,
                tracks: Vec::new(),
                curves: Vec::new(),
            });
            pending.clear();
            looping = TrackLoop::ONCE;
            at += 16;
            continue;
        }
        if at + PLAIN_CUE <= region.len()
            && w != 0
            && flags == 0
            && word(at + 16) == hash
            && (word(at + 20) as usize) < count
        {
            close(current.take(), &mut mixes);
            at += PLAIN_CUE;
            continue;
        }
        // A curve of the track to come.
        if (w == CURVE_VOLUME || w == CURVE_PITCH)
            && current.is_some()
            && flags == 0
            && (2..=32).contains(&word(at + 8))
            && word(at + 16) == 0x14
            && word(at + 20) == 0
        {
            let n = word(at + 8) as usize;
            let end = at + 24 + (2 * n - 1) * 4;
            if end <= region.len() {
                let mut points = vec![(0.0, float(at + 24))];
                points.extend((0..n - 1).map(|i| {
                    let p = at + 28 + i * 8;
                    (float(p), float(p + 4))
                }));
                pending.push(Curve {
                    kind: w,
                    param: word(at + 12),
                    points,
                });
                at = end;
                continue;
            }
        }
        if at >= 12
            && at + 12 <= region.len()
            && w != 0
            && word(at - 4) == 0x10
            && word(at - 12) == 1
            && let Some(m) = current.as_mut()
        {
            let start = float(at - 8);
            let weight = float(at + 8);
            if start.is_finite() && start >= 0.0 && weight.is_finite() {
                m.tracks.push(Track {
                    table: w,
                    record: word(at + 4),
                    start,
                    weight,
                    level: track_block(region, at - 16).unwrap_or(1.0),
                    looping,
                });
                m.curves.push(std::mem::take(&mut pending));
                at += 12;
                continue;
            }
        }
        at += 4;
    }
    close(current, &mut mixes);
    mixes
}

/// The value a mixed cue's track entry at `entry` (its first word) is
/// given by the block of eight words before it: `[0, 0, 0, 0, value, 1.0,
/// 0, 0]` (see [`MixedCue`]); `None` for none.
fn track_block(region: &[u8], entry: usize) -> Option<f32> {
    let block = region.get(entry.checked_sub(32)?..entry)?;
    let w = |i: usize| {
        u32::from_le_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ])
    };
    let value = f32::from_bits(w(4));
    ((0..4).all(|i| w(i) == 0)
        && f32::from_bits(w(5)) == 1.0
        && w(6) == 0
        && w(7) == 0
        && value > 0.0
        && value < 10.0)
        .then_some(value)
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
        // Its priority, and (a short record) its level at 0x2c.
        w[4] = 0.85f32.to_bits();
        w[11] = 0.7f32.to_bits();
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
        // A long record: its level drawn between 0x50 and 0x54.
        w[3] = 1;
        w[4] = 0.85f32.to_bits();
        w[20] = 0.5f32.to_bits();
        w[21] = 0.7f32.to_bits();
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
        // The middle of its range, not its priority.
        assert!((t.groups[0].volume - 0.6).abs() < 1e-6);
        // The record keeps the range, and a line's single level.
        let start = t.records[1].start;
        assert_eq!(start.level, (0.5, 0.7));
        assert!((start.level(0.25) - 0.55).abs() < 1e-6);
        assert!((start.level_scale(1.0) - 0.7 / 0.6).abs() < 1e-6);
        assert_eq!(t.records[0].start.level, (0.7, 0.7));
        assert_eq!(t.records[0].start.level_scale(0.9), 1.0);
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
    fn a_start_s_pitch_is_drawn_in_semitones() {
        let s = StartDraw {
            level: (1.0, 1.0),
            pitch: (-12.0, 12.0),
        };
        assert_eq!(s.rate(0.0), 0.5);
        assert_eq!(s.rate(0.5), 1.0);
        assert_eq!(s.rate(1.0), 2.0);
        assert_eq!(StartDraw::FLAT.rate(0.3), 1.0);
        assert_eq!(StartDraw::FLAT.level_scale(0.3), 1.0);
    }

    // An engine's start as the vehicles' tables lay it out: the record
    // word's high half set (0x3e99), its low half the record.
    #[test]
    fn a_plain_cues_record_is_its_last_words_low_half() {
        let mut d = table(&[
            line(1, RADIO, 10.0, 1000.0, 15),
            line(2, RADIO, 10.0, 1000.0, 16),
        ]);
        let list = d.len() as u32;
        let words: [u32; 6] = [
            0x5c55_1331,
            0,
            0x3f00_0000,
            0x4058_0000,
            0xabcd,
            0x3e99_0001,
        ];
        d.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        let end = d.len() as u32;
        d[0x18..0x1c].copy_from_slice(&list.to_le_bytes());
        d[0x1c..0x20].copy_from_slice(&end.to_le_bytes());
        let t = CueTable::parse_data(&d).unwrap();
        assert_eq!(
            t.cues,
            [PlainCue {
                cue: 0x5c55_1331,
                record: 1,
                a: 0.5,
                seconds: 3.375
            }]
        );
        assert_eq!(t.cue_record(0x5c55_1331).unwrap().volume, 0.7);
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

    // A mixed cue as the assault rifle's table lays its shot out (words
    // shaped as read on a legally installed copy; values made up): its
    // header, words not decoded, two tracks, then a plain cue ending it.
    // A HMMWV engine track's curves: speed moves its volume and pitch.
    #[test]
    fn a_tracks_curves_are_read_and_join_straight() {
        let mut d = table(&[
            line(1, RADIO, 10.0, 1000.0, 15),
            line(2, RADIO, 10.0, 1000.0, 16),
        ]);
        let list = d.len() as u32;
        let f = |v: f32| v.to_bits();
        let words: Vec<u32> = vec![
            0x1234_5678,
            0x100,
            f(0.84),
            f(-1.0), // a looping mixed cue
            CURVE_VOLUME,
            0,
            2,
            0x15ba_509e,
            0x14,
            0,
            f(0.0),
            f(1.0),
            f(1.0), // volume: 0 to 1
            CURVE_PITCH,
            0,
            2,
            0x15ba_509e,
            0x14,
            0,
            f(-1.0),
            f(0.5),
            f(3.0), // pitch: -1 to 3 by half speed
            0x102_0200,
            1,
            f(0.0),
            0x10,
            0xabcd,
            1,
            f(1.0), // track
            0x55,
            0,
            f(0.5),
            f(0.125),
            0xabcd,
            0, // a plain cue
        ];
        d.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        let end = d.len() as u32;
        d[0x18..0x1c].copy_from_slice(&list.to_le_bytes());
        d[0x1c..0x20].copy_from_slice(&end.to_le_bytes());
        let t = CueTable::parse_data(&d).unwrap();
        let m = t.mix(0x1234_5678).unwrap();
        assert_eq!(m.curves.len(), 1);
        let [volume, pitch] = &m.curves[0][..] else {
            panic!("two curves");
        };
        assert_eq!((volume.kind, pitch.kind), (CURVE_VOLUME, CURVE_PITCH));
        assert_eq!(volume.points, [(0.0, 0.0), (1.0, 1.0)]);
        assert_eq!(volume.at(0.25), 0.25);
        assert_eq!(pitch.at(0.25), 1.0);
        assert_eq!(pitch.at(0.75), 3.0);
    }

    // The Semi's engine start: a header block holding a track header
    // followed by 0x100, 1.0 and -1, which reads like a looping cue of
    // its own; the cue is still found, with its one track.
    #[test]
    fn a_track_header_in_a_cues_header_is_not_a_cue() {
        let mut d = table(&[
            line(1, RADIO, 10.0, 1000.0, 15),
            line(2, RADIO, 10.0, 1000.0, 16),
        ]);
        let list = d.len() as u32;
        let f = |v: f32| v.to_bits();
        let words: Vec<u32> = vec![
            0x9197_0795,
            0x2_0100,
            f(0.631),
            f(1.64), // the mixed cue
            0x1_0000,
            0x100,
            f(1.0),
            f(-1.0), // a track header, not a cue
            0x1c,
            0x102_0200,
            1,
            f(0.0),
            0x10,
            0xabcd,
            1,
            f(1.0), // track
            0x55,
            0,
            f(0.5),
            f(0.125),
            0xabcd,
            0, // a plain cue
        ];
        d.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        let end = d.len() as u32;
        d[0x18..0x1c].copy_from_slice(&list.to_le_bytes());
        d[0x1c..0x20].copy_from_slice(&end.to_le_bytes());
        let t = CueTable::parse_data(&d).unwrap();
        let m = t.mix(0x9197_0795).unwrap();
        assert_eq!((m.seconds, m.tracks.len()), (1.64, 1));
        assert_eq!(m.tracks[0].record, 1);
        assert_eq!(t.mixes.len(), 1);
    }

    // A looped mix as the ambience table has them: a bed track that
    // plays through, and a one-shot track looping over its own span.
    #[test]
    fn a_track_carries_its_loop() {
        let mut d = table(&[
            line(1, RADIO, 10.0, 1000.0, 15),
            line(2, RADIO, 10.0, 1000.0, 16),
        ]);
        let list = d.len() as u32;
        let f = |v: f32| v.to_bits();
        let words: Vec<u32> = vec![
            0x9999_0077,
            0x100,
            f(0.316),
            f(-1.0), // the mixed cue, looping
            0x2_0000,
            0x200,
            f(1.0),
            f(-1.0),
            f(-1.0),
            0,
            0x44,
            0x44,
            // The bed's track: one record, no loop.
            0x1_0000,
            f(-1.0),
            f(-1.0),
            0x1c,
            0x1c,
            0x1c,
            0x38,
            0x102_0200,
            1,
            f(0.0),
            0x10,
            0xabcd,
            0,
            f(1.0),
            0,
            // A one-shot's: for ever over 0 to 23.2 s, its record 5.11 s in.
            0x1_01ff,
            f(0.0),
            f(23.2),
            0x1c,
            0x64,
            0x68,
            0x84,
            0x102_0201,
            1,
            f(5.11),
            0x10,
            0xabcd,
            1,
            f(0.25),
            0,
        ];
        d.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        let end = d.len() as u32;
        d[0x18..0x1c].copy_from_slice(&list.to_le_bytes());
        d[0x1c..0x20].copy_from_slice(&end.to_le_bytes());
        let t = CueTable::parse_data(&d).unwrap();
        let m = t.mix(0x9999_0077).unwrap();
        assert_eq!(m.tracks.len(), 2);
        assert_eq!(m.tracks[0].looping, TrackLoop::ONCE);
        let shot = m.tracks[1];
        assert_eq!((shot.record, shot.start, shot.weight), (1, 5.11, 0.25));
        assert_eq!(
            shot.looping,
            TrackLoop {
                passes: 0xff,
                start: 0.0,
                end: 23.2
            }
        );
        assert!(shot.looping.is_endless());
    }

    #[test]
    fn finds_mixed_cues_and_their_tracks() {
        let mut d = table(&[
            line(1, RADIO, 10.0, 1000.0, 15),
            line(2, RADIO, 10.0, 1000.0, 16),
        ]);
        let list = d.len() as u32;
        let f = |v: f32| v.to_bits();
        let words: Vec<u32> = vec![
            0x9999_0099,
            0x100,
            f(0.75),
            f(1.35), // the mixed cue
            0x6_0000,
            0x600,
            f(1.0),
            f(-1.0),
            0x44,
            0x1ac, // not decoded
            0x102_0200,
            1,
            f(0.0),
            0x10,
            0xabcd,
            1,
            f(1.0),
            0, // track
            0x1_0000,
            f(-1.0),
            0x1c, // not decoded
            0x102_0205,
            1,
            f(0.5667),
            0x10,
            0xabcd,
            0,
            f(1.0),
            0, // track
            0x55,
            0,
            f(0.5),
            f(0.125),
            0xabcd,
            1, // a plain cue
            0x102_0200,
            1,
            f(0.0),
            0x10,
            0xabcd,
            0,
            f(1.0), // after it: none
        ];
        d.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        let end = d.len() as u32;
        d[0x18..0x1c].copy_from_slice(&list.to_le_bytes());
        d[0x1c..0x20].copy_from_slice(&end.to_le_bytes());
        let t = CueTable::parse_data(&d).unwrap();
        assert_eq!(t.cues.len(), 1);
        let m = t.mix(0x9999_0099).unwrap();
        assert_eq!(m.seconds, 1.35);
        assert_eq!(
            m.tracks,
            [
                Track {
                    table: 0xabcd,
                    record: 1,
                    start: 0.0,
                    weight: 1.0,
                    level: 1.0,
                    looping: TrackLoop::ONCE,
                },
                Track {
                    table: 0xabcd,
                    record: 0,
                    start: 0.5667,
                    weight: 1.0,
                    level: 1.0,
                    looping: TrackLoop::ONCE,
                }
            ]
        );
        assert_eq!(t.mixes.len(), 1);
        assert!(t.mix(0x55).is_none());
    }

    // The pistol's shot: a track whose header has 0x100 set carries a
    // block giving it a level (0.179 as read; words shaped as read on a
    // legally installed copy). A looping cue (the minigun's, -1 s) is a mix
    // too.
    #[test]
    fn a_tracks_block_sets_its_level_and_looping_mixes_are_found() {
        let mut d = table(&[line(1, RADIO, 10.0, 1000.0, 15)]);
        let list = d.len() as u32;
        let f = |v: f32| v.to_bits();
        let words: Vec<u32> = vec![
            0x7777_0077,
            0x100,
            f(0.631),
            f(2.537), // the mixed cue
            0x1_0100,
            f(-1.0),
            f(-1.0),
            0x1c,
            0x38,
            0x3c,
            0x58, // a track header with a block
            0,
            0,
            0,
            0,
            f(0.179),
            f(1.0),
            0,
            0, // the block
            0x202_0203,
            1,
            f(0.0),
            0x10,
            0xabcd,
            0,
            f(1.0),
            0, // its entry
            0x8888_0088,
            0x100,
            f(0.28),
            f(-1.0), // a loop
            0x102_0200,
            1,
            f(0.0333),
            0x10,
            0xabcd,
            0,
            f(1.0),
            0,
        ];
        d.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        let end = d.len() as u32;
        d[0x18..0x1c].copy_from_slice(&list.to_le_bytes());
        d[0x1c..0x20].copy_from_slice(&end.to_le_bytes());
        let t = CueTable::parse_data(&d).unwrap();
        let shot = t.mix(0x7777_0077).unwrap();
        assert_eq!(shot.tracks.len(), 1);
        assert_eq!(shot.tracks[0].start, 0.0);
        assert!((shot.tracks[0].level - 0.179).abs() < 1e-6);
        let spin = t.mix(0x8888_0088).unwrap();
        assert_eq!(spin.seconds, -1.0);
        assert!((spin.tracks[0].start - 0.0333).abs() < 1e-6);
        assert_eq!(spin.tracks[0].level, 1.0);
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
