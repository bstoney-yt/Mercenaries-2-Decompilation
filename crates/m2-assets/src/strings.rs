//! String tables: the game's text for one language, keyed by
//! [`name_hash`] of the text's id (`VzaCon001.Objectives.001`, as the
//! scripts write it in `[...]`).
//!
//! The English table is `blocks\VZ\english_P000_Q3.block` in `vz.wad` (and
//! the same in `shell.wad`): a `UCFX` container of type `39e5e978` with
//! three sections:
//!
//! ```text
//! INFO  u32 5, then 4 bytes (unknown)
//! KEYS  u32 count, then count × (u32 key hash, u32 byte offset into STRS
//!       after its first u32)
//! STRS  u32 total UTF-16 units, then null-terminated UTF-16LE strings
//! ```
//!
//! # Provenance
//!
//! Format-from-samples: the English table of a legally installed copy
//! (18,299 strings). No text from the game is committed.

use std::collections::HashMap;

use crate::bytes::{OutOfBounds, Reader};
use crate::hash::name_hash;
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of string tables.
pub const STRINGS_TYPE: u32 = 0x39e5_e978;

/// Errors from parsing a string table.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum StringsError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("no {0} section")]
    Missing(&'static str),
    #[error("string {key:08x} starts at {offset:#x}, past the strings")]
    Offset { key: u32, offset: usize },
}

impl From<OutOfBounds> for StringsError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// A language's strings by key hash.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StringTable {
    pub strings: HashMap<u32, String>,
}

impl StringTable {
    /// Parses a string table asset.
    pub fn parse(asset: &[u8]) -> Result<Self, StringsError> {
        let ucfx = Ucfx::parse(asset)?;
        let keys = ucfx
            .top_level(*b"KEYS")
            .ok_or(StringsError::Missing("KEYS"))?;
        let strs = ucfx
            .top_level(*b"STRS")
            .ok_or(StringsError::Missing("STRS"))?;
        Self::parse_sections(keys, strs)
    }

    /// Parses the `KEYS` and `STRS` sections.
    pub fn parse_sections(keys: &[u8], strs: &[u8]) -> Result<Self, StringsError> {
        let mut r = Reader::at(keys, 0);
        let count = r.u32()? as usize;
        let text = strs.get(4..).unwrap_or_default();
        let mut strings = HashMap::with_capacity(count);
        for _ in 0..count {
            let (key, offset) = (r.u32()?, r.u32()? as usize);
            let rest = text
                .get(offset..)
                .ok_or(StringsError::Offset { key, offset })?;
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            strings.insert(key, String::from_utf16_lossy(&units));
        }
        Ok(Self { strings })
    }

    pub fn get(&self, key: u32) -> Option<&str> {
        self.strings.get(&key).map(String::as_str)
    }

    /// The text with id `id` (`VzaCon001.Objectives.001`, any case).
    pub fn lookup(&self, id: &str) -> Option<&str> {
        self.get(name_hash(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    #[test]
    fn keys_point_at_strings() {
        let (a, b) = (utf16("Variant"), utf16("Done"));
        let mut strs = 6u32.to_le_bytes().to_vec();
        strs.extend(&a);
        strs.extend(&b);
        let mut keys = 2u32.to_le_bytes().to_vec();
        for (k, o) in [(0x1111_1111u32, 0u32), (0x2222_2222, a.len() as u32)] {
            keys.extend(k.to_le_bytes());
            keys.extend(o.to_le_bytes());
        }
        let asset = ucfx(&[
            UcfxNode::Leaf(*b"INFO", vec![5, 0, 0, 0, 0, 0, 0, 0]),
            UcfxNode::Leaf(*b"KEYS", keys),
            UcfxNode::Leaf(*b"STRS", strs),
        ]);
        let t = StringTable::parse(&asset).unwrap();
        assert_eq!(t.get(0x1111_1111), Some("Variant"));
        assert_eq!(t.get(0x2222_2222), Some("Done"));
        assert_eq!(t.get(3), None);
    }

    #[test]
    fn ids_are_looked_up_by_their_hash() {
        let mut t = StringTable::default();
        t.strings.insert(
            name_hash("objective.Objectivecompleted"),
            "Objective completed:".into(),
        );
        assert_eq!(
            t.lookup("Objective.ObjectiveCompleted"),
            Some("Objective completed:")
        );
        assert_eq!(t.lookup("objective.other"), None);
    }

    #[test]
    fn offsets_past_the_end_are_errors() {
        let mut keys = 1u32.to_le_bytes().to_vec();
        keys.extend(7u32.to_le_bytes());
        keys.extend(100u32.to_le_bytes());
        assert_eq!(
            StringTable::parse_sections(&keys, &[0; 8]),
            Err(StringsError::Offset {
                key: 7,
                offset: 100
            })
        );
    }
}
