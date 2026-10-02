//! The `sges` block container.
//!
//! Every block in a `.wad` archive is stored as one `sges` container: a
//! 16-byte header, a table of 8-byte segment records, then the segment data.
//! Each segment holds at most 64 KiB of output and is either raw DEFLATE (no
//! zlib header) or stored as-is. Concatenating the segments in table order
//! gives the block's contents.
//!
//! ```text
//! 0x00  [u8; 4]  magic "sges"
//! 0x04  u16      version (4)
//! 0x06  u16      segment count
//! 0x08  u32      total decoded length
//! 0x0C  u32      total stored length (meaning not verified)
//! 0x10  segment records, 8 bytes each:
//!       u16  stored length    (0 = 0x10000)
//!       u16  decoded length   (0 = 0x10000)
//!       u32  offset from the start of the container; bit 0 set = compressed
//! ```
//!
//! Bytes between the segment table and the first segment are padding and
//! may hold stale data; they are ignored.

use crate::bytes::{OutOfBounds, Reader};

/// Container magic.
pub const MAGIC: [u8; 4] = *b"sges";
/// The only container version seen.
pub const VERSION: u16 = 4;
/// Largest decoded size of a single segment.
pub const MAX_SEGMENT_LEN: u32 = 0x1_0000;
/// Largest decoded block this parser will allocate for.
pub const MAX_BLOCK_LEN: u64 = 1 << 30;

const HEADER_LEN: usize = 0x10;
const SEGMENT_RECORD_LEN: usize = 8;

/// Errors from parsing or decoding an `sges` container.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SgesError {
    #[error("not an sges container (magic {0:02x?})")]
    BadMagic([u8; 4]),
    #[error("unsupported sges version {0}")]
    UnsupportedVersion(u16),
    #[error("container truncated: needed {len} bytes at offset {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("segment {index} ({offset:#x}+{len:#x}) lies outside the {size:#x}-byte container")]
    SegmentOutOfBounds {
        index: usize,
        offset: u32,
        len: u32,
        size: usize,
    },
    #[error("segment lengths add up to {actual:#x}, header says {expected:#x}")]
    LengthMismatch { expected: u64, actual: u64 },
    #[error("decoded block length {0:#x} exceeds the {MAX_BLOCK_LEN:#x}-byte limit")]
    TooLarge(u64),
    #[error("stored segment {index} has stored length {stored:#x} but decoded length {len:#x}")]
    StoredLengthMismatch { index: usize, stored: u32, len: u32 },
    #[error("segment {index} failed to inflate")]
    Inflate { index: usize },
    #[error("segment {index} inflated to {actual:#x} bytes, expected {expected:#x}")]
    InflatedLength {
        index: usize,
        expected: u32,
        actual: usize,
    },
}

impl From<OutOfBounds> for SgesError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One segment record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// Offset of the segment data from the start of the container.
    pub offset: u32,
    /// Bytes stored on disk.
    pub stored_len: u32,
    /// Bytes after decoding.
    pub len: u32,
    /// Raw DEFLATE if set, otherwise stored.
    pub compressed: bool,
}

/// A parsed container header and segment table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SgesHeader {
    pub version: u16,
    /// Total decoded length; equals the sum of the segment lengths.
    pub len: u32,
    /// Header field at 0x0C. Looks like the stored length; not verified.
    pub unk_0x0c: u32,
    pub segments: Vec<Segment>,
}

/// A length field where 0 stands for 0x10000.
fn len16(v: u16) -> u32 {
    if v == 0 {
        MAX_SEGMENT_LEN
    } else {
        u32::from(v)
    }
}

impl SgesHeader {
    /// Parses the header and segment table and checks every segment lies
    /// inside `buf` and the lengths agree.
    pub fn parse(buf: &[u8]) -> Result<Self, SgesError> {
        let mut r = Reader::at(buf, 0);
        let magic = r.array::<4>()?;
        if magic != MAGIC {
            return Err(SgesError::BadMagic(magic));
        }
        let version = r.u16()?;
        if version != VERSION {
            return Err(SgesError::UnsupportedVersion(version));
        }
        let count = usize::from(r.u16()?);
        let len = r.u32()?;
        let unk_0x0c = r.u32()?;
        debug_assert_eq!(r.pos(), HEADER_LEN);

        // Check the whole table is present before allocating for it.
        Reader::at(buf, HEADER_LEN).take(count * SEGMENT_RECORD_LEN)?;
        let mut segments = Vec::with_capacity(count);
        let mut total = 0u64;
        for index in 0..count {
            let stored_len = len16(r.u16()?);
            let seg_len = len16(r.u16()?);
            let word = r.u32()?;
            let seg = Segment {
                offset: word & !1,
                stored_len,
                len: seg_len,
                compressed: word & 1 != 0,
            };
            let end = u64::from(seg.offset) + u64::from(seg.stored_len);
            if end > buf.len() as u64 {
                return Err(SgesError::SegmentOutOfBounds {
                    index,
                    offset: seg.offset,
                    len: seg.stored_len,
                    size: buf.len(),
                });
            }
            if !seg.compressed && seg.stored_len != seg.len {
                return Err(SgesError::StoredLengthMismatch {
                    index,
                    stored: seg.stored_len,
                    len: seg.len,
                });
            }
            total += u64::from(seg.len);
            segments.push(seg);
        }
        if total != u64::from(len) {
            return Err(SgesError::LengthMismatch {
                expected: len.into(),
                actual: total,
            });
        }
        if total > MAX_BLOCK_LEN {
            return Err(SgesError::TooLarge(total));
        }
        Ok(Self {
            version,
            len,
            unk_0x0c,
            segments,
        })
    }
}

/// Decodes a whole container to the block's contents.
pub fn decode(buf: &[u8]) -> Result<Vec<u8>, SgesError> {
    let header = SgesHeader::parse(buf)?;
    // `parse` capped the length at MAX_BLOCK_LEN, which fits in usize.
    let mut out = Vec::with_capacity(header.len as usize);
    for (index, seg) in header.segments.iter().enumerate() {
        let start = seg.offset as usize;
        let data = &buf[start..start + seg.stored_len as usize];
        if seg.compressed {
            let expected = seg.len as usize;
            let inflated = miniz_oxide::inflate::decompress_to_vec_with_limit(data, expected)
                .map_err(|_| SgesError::Inflate { index })?;
            if inflated.len() != expected {
                return Err(SgesError::InflatedLength {
                    index,
                    expected: seg.len,
                    actual: inflated.len(),
                });
            }
            out.extend_from_slice(&inflated);
        } else {
            out.extend_from_slice(data);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{SegmentKind, sges};

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i / 7) as u8 ^ seed).collect()
    }

    #[test]
    fn decodes_mixed_segments() {
        let a = pattern(0x1_0000, 1);
        let b = pattern(0x1234, 2);
        let c = pattern(0x99, 3);
        let buf = sges(&[
            (&a, SegmentKind::Deflate),
            (&b, SegmentKind::Stored),
            (&c, SegmentKind::Deflate),
        ]);
        let header = SgesHeader::parse(&buf).unwrap();
        assert_eq!(header.segments.len(), 3);
        assert_eq!(header.segments[0].len, 0x1_0000);
        assert!(header.segments[0].compressed);
        assert!(!header.segments[1].compressed);
        assert!(header.segments.iter().all(|s| s.offset % 16 == 0));

        let out = decode(&buf).unwrap();
        assert_eq!(out, [a, b, c].concat());
    }

    #[test]
    fn empty_container_decodes_to_nothing() {
        assert_eq!(decode(&sges(&[])).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn stored_full_segment_uses_zero_lengths() {
        let a = pattern(0x1_0000, 9);
        let buf = sges(&[(&a, SegmentKind::Stored)]);
        assert_eq!(&buf[0x10..0x14], &[0, 0, 0, 0]);
        assert_eq!(decode(&buf).unwrap(), a);
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        let mut buf = sges(&[]);
        buf[4] = 5;
        assert_eq!(decode(&buf), Err(SgesError::UnsupportedVersion(5)));
        buf[0] = b'x';
        assert!(matches!(decode(&buf), Err(SgesError::BadMagic(_))));
    }

    #[test]
    fn rejects_truncation_everywhere() {
        let a = pattern(0x500, 4);
        let buf = sges(&[(&a, SegmentKind::Deflate), (&a, SegmentKind::Stored)]);
        for len in 0..buf.len() {
            // Trailing alignment padding is not part of any segment.
            let last = SgesHeader::parse(&buf).unwrap().segments[1];
            if len >= (last.offset + last.stored_len) as usize {
                continue;
            }
            assert!(decode(&buf[..len]).is_err(), "accepted {len} bytes");
        }
    }

    #[test]
    fn rejects_length_mismatch() {
        let a = pattern(0x40, 5);
        let mut buf = sges(&[(&a, SegmentKind::Stored)]);
        buf[8] = 0x41;
        assert_eq!(
            decode(&buf),
            Err(SgesError::LengthMismatch {
                expected: 0x41,
                actual: 0x40
            })
        );
    }

    #[test]
    fn rejects_corrupt_deflate() {
        let a = pattern(0x800, 6);
        let mut buf = sges(&[(&a, SegmentKind::Deflate)]);
        // Truncate the stream inside the record so inflate runs out of input.
        buf[0x10..0x12].copy_from_slice(&2u16.to_le_bytes());
        assert!(matches!(
            decode(&buf),
            Err(SgesError::Inflate { index: 0 } | SgesError::InflatedLength { index: 0, .. })
        ));
    }

    #[test]
    fn rejects_stored_segment_with_unequal_lengths() {
        let a = pattern(0x40, 7);
        let mut buf = sges(&[(&a, SegmentKind::Stored)]);
        buf[0x10..0x12].copy_from_slice(&0x30u16.to_le_bytes());
        assert!(matches!(
            decode(&buf),
            Err(SgesError::StoredLengthMismatch { index: 0, .. })
        ));
    }

    #[test]
    fn rejects_huge_segment_count_without_allocating() {
        let mut buf = sges(&[]);
        buf[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(decode(&buf), Err(SgesError::Truncated { .. })));
    }
}
