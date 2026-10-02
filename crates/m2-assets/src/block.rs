//! The asset directory at the start of a decoded block.
//!
//! ```text
//! 0x00  u32  entry count
//! 0x04  entries, 16 bytes each:
//!       u32  asset name hash
//!       u32  type-name hash (one of the archive's type hashes)
//!       u32  unknown; nonzero only on entries reached through
//!            `Asset::alt_blocks`, and smaller than the asset
//!       u32  asset length in bytes
//! then the assets, packed back to back in entry order with no padding.
//! ```
//!
//! The asset lengths add up to the rest of the block exactly. An asset hash
//! can repeat within a block with a different type, so assets are keyed by
//! `(name_hash, type_hash)`.

use crate::bytes::{OutOfBounds, Reader};

const ENTRY_LEN: usize = 16;

/// Errors from parsing a block directory.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BlockError {
    #[error("block directory truncated: needed {len} bytes at offset {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("asset lengths add up to {actual:#x} bytes, but {expected:#x} follow the directory")]
    LengthMismatch { expected: usize, actual: u64 },
}

impl From<OutOfBounds> for BlockError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One directory entry, with the asset's position in the decoded block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockEntry {
    pub name_hash: u32,
    pub type_hash: u32,
    /// Directory field at 0x08; see the module docs.
    pub unk_0x08: u32,
    /// Offset of the asset in the decoded block.
    pub offset: usize,
    pub len: usize,
}

/// A decoded block split into its assets.
#[derive(Debug, Clone)]
pub struct BlockContents<'a> {
    data: &'a [u8],
    entries: Vec<BlockEntry>,
}

impl<'a> BlockContents<'a> {
    /// Parses the directory of a decoded block.
    pub fn parse(data: &'a [u8]) -> Result<Self, BlockError> {
        let mut r = Reader::at(data, 0);
        let count = r.u32()? as usize;
        // Check the whole directory is present before allocating for it.
        let dir_len = count
            .checked_mul(ENTRY_LEN)
            .and_then(|n| n.checked_add(4))
            .ok_or(BlockError::Truncated {
                offset: 4,
                len: usize::MAX,
            })?;
        Reader::at(data, 4).take(dir_len - 4)?;

        let mut entries = Vec::with_capacity(count);
        let mut offset = dir_len as u64;
        for _ in 0..count {
            let name_hash = r.u32()?;
            let type_hash = r.u32()?;
            let unk_0x08 = r.u32()?;
            let len = r.u32()?;
            entries.push(BlockEntry {
                name_hash,
                type_hash,
                unk_0x08,
                // Checked against data.len() below, so both fit in usize.
                offset: offset as usize,
                len: len as usize,
            });
            offset += u64::from(len);
        }
        if offset != data.len() as u64 {
            return Err(BlockError::LengthMismatch {
                expected: data.len() - dir_len,
                actual: offset - dir_len as u64,
            });
        }
        Ok(Self { data, entries })
    }

    pub fn entries(&self) -> &[BlockEntry] {
        &self.entries
    }

    /// The bytes of one asset.
    pub fn asset_data(&self, entry: &BlockEntry) -> &'a [u8] {
        &self.data[entry.offset..entry.offset + entry.len]
    }

    /// Finds an asset by name and type hash.
    pub fn find(&self, name_hash: u32, type_hash: u32) -> Option<&BlockEntry> {
        self.entries
            .iter()
            .find(|e| e.name_hash == name_hash && e.type_hash == type_hash)
    }

    /// Each entry with its bytes.
    pub fn iter(&self) -> impl Iterator<Item = (&BlockEntry, &'a [u8])> + '_ {
        self.entries.iter().map(|e| (e, self.asset_data(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::block_contents;

    #[test]
    fn splits_assets_in_order() {
        let data = block_contents(&[
            (1, 0xA, 0, b"first".to_vec()),
            (1, 0xB, 7, b"odd".to_vec()),
            (2, 0xA, 0, Vec::new()),
        ]);
        let block = BlockContents::parse(&data).unwrap();
        let got: Vec<_> = block.iter().map(|(e, d)| (e.name_hash, d)).collect();
        assert_eq!(got, [(1, &b"first"[..]), (1, b"odd"), (2, b"")]);
        assert_eq!(block.entries()[1].unk_0x08, 7);
        assert_eq!(block.entries()[0].offset, 4 + 3 * 16);
        assert_eq!(block.find(1, 0xB).map(|e| e.len), Some(3));
        assert!(block.find(2, 0xB).is_none());
    }

    #[test]
    fn rejects_lengths_that_do_not_fill_the_block() {
        let mut data = block_contents(&[(1, 0xA, 0, b"abcd".to_vec())]);
        data.push(0);
        assert_eq!(
            BlockContents::parse(&data).unwrap_err(),
            BlockError::LengthMismatch {
                expected: 5,
                actual: 4
            }
        );
        data.truncate(data.len() - 2);
        assert!(matches!(
            BlockContents::parse(&data),
            Err(BlockError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn rejects_truncated_or_huge_directories() {
        let data = block_contents(&[(1, 0xA, 0, b"abcd".to_vec())]);
        for len in 0..20 {
            assert!(BlockContents::parse(&data[..len]).is_err(), "{len}");
        }
        let mut huge = data.clone();
        huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            BlockContents::parse(&huge),
            Err(BlockError::Truncated { .. })
        ));
        // Lengths that overflow a u32 sum.
        let mut over = block_contents(&[(1, 0xA, 0, Vec::new()), (2, 0xA, 0, Vec::new())]);
        over[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        over[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            BlockContents::parse(&over),
            Err(BlockError::LengthMismatch { .. })
        ));
    }
}
