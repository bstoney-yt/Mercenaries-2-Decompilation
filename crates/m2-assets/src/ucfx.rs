//! The `UCFX` container every asset is stored in.
//!
//! ```text
//! 0x00  [u8; 4]  magic "UCFX"
//! 0x04  u32      header length (0x14 + 20 * section count)
//! 0x08  u32      unknown (0)
//! 0x0C  u32      unknown (0)
//! 0x10  u32      section count
//! 0x14  section records, 20 bytes each:
//!       [u8; 4]  fourcc
//!       u32      offset from the end of the header (0xFFFFFFFF on containers)
//!       u32      length (0 on containers)
//!       u32      number of siblings after this one at the same level
//!       u32      number of direct children (nonzero = container)
//! then the leaf sections' data, followed by "CSUM" and a u32 whose meaning
//! is unknown.
//! ```
//!
//! The records form a tree written depth-first: a container's children
//! follow it directly. The asset's name, where it has one, is the
//! NUL-terminated string in the top-level `NAME` section; so far only
//! texture assets carry one.

use crate::bytes::{OutOfBounds, Reader};

/// Container magic.
pub const MAGIC: [u8; 4] = *b"UCFX";
/// Section holding the asset's name.
pub const NAME: [u8; 4] = *b"NAME";
/// Deepest section nesting accepted. Real assets nest four or five deep.
pub const MAX_DEPTH: usize = 32;

const HEADER_LEN: usize = 0x14;
const RECORD_LEN: usize = 20;

/// Errors from parsing a `UCFX` container.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UcfxError {
    #[error("not a UCFX container (magic {0:02x?})")]
    BadMagic([u8; 4]),
    #[error("container truncated: needed {len} bytes at offset {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("header length {len:#x} does not match {count} sections")]
    HeaderLength { len: u32, count: u32 },
    #[error("section {index} has {found} siblings after it, expected {expected}")]
    BadSiblingCount {
        index: usize,
        expected: u32,
        found: u32,
    },
    #[error("section tree uses {used} of {count} records")]
    RecordCount { used: usize, count: usize },
    #[error("sections nest deeper than {MAX_DEPTH} levels")]
    TooDeep,
    #[error("section {index} ({offset:#x}+{len:#x}) lies outside the {size:#x}-byte payload")]
    SectionOutOfBounds {
        index: usize,
        offset: u32,
        len: u32,
        size: usize,
    },
}

impl From<OutOfBounds> for UcfxError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One section, in depth-first order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section<'a> {
    pub fourcc: [u8; 4],
    /// 0 for top-level sections.
    pub depth: usize,
    /// Number of direct children; nonzero for containers.
    pub children: u32,
    /// The section's bytes; `None` for containers.
    pub data: Option<&'a [u8]>,
}

/// A parsed `UCFX` container.
#[derive(Debug, Clone)]
pub struct Ucfx<'a> {
    sections: Vec<Section<'a>>,
    payload: &'a [u8],
}

struct Raw {
    fourcc: [u8; 4],
    offset: u32,
    len: u32,
    siblings_after: u32,
    children: u32,
}

impl<'a> Ucfx<'a> {
    /// Parses an asset's container and checks every leaf section lies inside
    /// the asset.
    pub fn parse(asset: &'a [u8]) -> Result<Self, UcfxError> {
        let mut r = Reader::at(asset, 0);
        let magic = r.array::<4>()?;
        if magic != MAGIC {
            return Err(UcfxError::BadMagic(magic));
        }
        let header_len = r.u32()?;
        r.u32()?;
        r.u32()?;
        let count = r.u32()?;
        if u64::from(header_len) != HEADER_LEN as u64 + u64::from(count) * RECORD_LEN as u64 {
            return Err(UcfxError::HeaderLength {
                len: header_len,
                count,
            });
        }
        // Bounded by the check above and present in `asset` after this take.
        let header = Reader::at(asset, 0).take(header_len as usize)?;
        let payload = &asset[header.len()..];

        let raws = (0..count)
            .map(|_| {
                Ok(Raw {
                    fourcc: r.array()?,
                    offset: r.u32()?,
                    len: r.u32()?,
                    siblings_after: r.u32()?,
                    children: r.u32()?,
                })
            })
            .collect::<Result<Vec<_>, OutOfBounds>>()?;

        let mut sections = Vec::with_capacity(raws.len());
        // Each frame: sections still expected at that level.
        let mut stack: Vec<u32> = Vec::new();
        if let Some(first) = raws.first() {
            stack.push(first.siblings_after.saturating_add(1));
        }
        for (index, raw) in raws.iter().enumerate() {
            let Some(remaining) = stack.last_mut() else {
                return Err(UcfxError::RecordCount {
                    used: index,
                    count: raws.len(),
                });
            };
            if raw.siblings_after != *remaining - 1 {
                return Err(UcfxError::BadSiblingCount {
                    index,
                    expected: *remaining - 1,
                    found: raw.siblings_after,
                });
            }
            *remaining -= 1;
            let depth = stack.len() - 1;
            let data = if raw.children == 0 {
                let end = u64::from(raw.offset) + u64::from(raw.len);
                if end > payload.len() as u64 {
                    return Err(UcfxError::SectionOutOfBounds {
                        index,
                        offset: raw.offset,
                        len: raw.len,
                        size: payload.len(),
                    });
                }
                Some(&payload[raw.offset as usize..end as usize])
            } else {
                None
            };
            sections.push(Section {
                fourcc: raw.fourcc,
                depth,
                children: raw.children,
                data,
            });
            if raw.children > 0 {
                if stack.len() == MAX_DEPTH {
                    return Err(UcfxError::TooDeep);
                }
                stack.push(raw.children);
            }
            while stack.last() == Some(&0) {
                stack.pop();
            }
        }
        if !stack.is_empty() {
            return Err(UcfxError::RecordCount {
                used: raws.len(),
                count: stack
                    .iter()
                    .map(|&n| n as usize)
                    .fold(raws.len(), usize::saturating_add),
            });
        }
        Ok(Self { sections, payload })
    }

    /// All sections in depth-first order.
    pub fn sections(&self) -> &[Section<'a>] {
        &self.sections
    }

    /// The bytes after the header: leaf data, then the trailing checksum.
    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    /// The first top-level leaf section with this fourcc.
    pub fn top_level(&self, fourcc: [u8; 4]) -> Option<&'a [u8]> {
        self.sections
            .iter()
            .find(|s| s.depth == 0 && s.fourcc == fourcc)
            .and_then(|s| s.data)
    }

    /// The asset's name from its top-level `NAME` section, cut at the first
    /// NUL. `None` if there is no such section or it is not UTF-8.
    pub fn name(&self) -> Option<&'a str> {
        let raw = self.top_level(NAME)?;
        let raw = raw.split(|&b| b == 0).next().unwrap_or_default();
        std::str::from_utf8(raw).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn leaf(cc: &[u8; 4], data: &[u8]) -> UcfxNode {
        UcfxNode::Leaf(*cc, data.to_vec())
    }

    #[test]
    fn reads_name_and_sections() {
        let asset = ucfx(&[
            leaf(b"NAME", b"global_loading_skull\0"),
            leaf(b"INFO", &[1, 2, 3]),
            leaf(b"BODY", &[9; 0x40]),
        ]);
        let u = Ucfx::parse(&asset).unwrap();
        assert_eq!(u.name(), Some("global_loading_skull"));
        assert_eq!(u.sections().len(), 3);
        assert_eq!(u.top_level(*b"INFO"), Some(&[1, 2, 3][..]));
        assert!(u.payload().ends_with(&[b'C', b'S', b'U', b'M', 0, 0, 0, 0]));
    }

    #[test]
    fn walks_nested_containers() {
        let asset = ucfx(&[
            UcfxNode::Container(
                *b"COMP",
                vec![
                    leaf(b"info", b"i"),
                    UcfxNode::Container(*b"schm", vec![leaf(b"NAME", b"bin"), leaf(b"decl", b"")]),
                    leaf(b"data", b"dd"),
                ],
            ),
            leaf(b"BODY", b"body"),
        ]);
        let u = Ucfx::parse(&asset).unwrap();
        let shape: Vec<_> = u
            .sections()
            .iter()
            .map(|s| (std::str::from_utf8(&s.fourcc).unwrap(), s.depth))
            .collect();
        assert_eq!(
            shape,
            [
                ("COMP", 0),
                ("info", 1),
                ("schm", 1),
                ("NAME", 2),
                ("decl", 2),
                ("data", 1),
                ("BODY", 0)
            ]
        );
        // A nested NAME is not the asset's name.
        assert_eq!(u.name(), None);
        assert_eq!(u.top_level(*b"BODY"), Some(&b"body"[..]));
        assert_eq!(u.top_level(*b"COMP"), None);
    }

    #[test]
    fn empty_container_has_no_sections() {
        let asset = ucfx(&[]);
        let u = Ucfx::parse(&asset).unwrap();
        assert!(u.sections().is_empty());
        assert_eq!(u.name(), None);
    }

    #[test]
    fn rejects_bad_headers() {
        let good = ucfx(&[leaf(b"NAME", b"a\0")]);
        let mut a = good.clone();
        a[0] = b'X';
        assert!(matches!(Ucfx::parse(&a), Err(UcfxError::BadMagic(_))));
        let mut a = good.clone();
        a[4] += 1;
        assert!(matches!(
            Ucfx::parse(&a),
            Err(UcfxError::HeaderLength { .. })
        ));
        let mut a = good.clone();
        a[0x10..0x14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            Ucfx::parse(&a),
            Err(UcfxError::HeaderLength { .. })
        ));
        for len in 0..0x28 {
            assert!(Ucfx::parse(&good[..len]).is_err(), "{len}");
        }
    }

    #[test]
    fn rejects_inconsistent_trees() {
        let good = ucfx(&[
            UcfxNode::Container(*b"COMP", vec![leaf(b"info", b"i")]),
            leaf(b"BODY", b"b"),
        ]);
        assert!(Ucfx::parse(&good).is_ok());
        // Records: COMP at 0x14, info at 0x28, BODY at 0x3c.
        let mut a = good.clone();
        a[0x14 + 16..0x14 + 20].copy_from_slice(&2u32.to_le_bytes());
        assert!(Ucfx::parse(&a).is_err());
        let mut a = good.clone();
        a[0x3c + 12..0x3c + 16].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            Ucfx::parse(&a),
            Err(UcfxError::BadSiblingCount { index: 2, .. })
        ));
        let mut a = good;
        a[0x28 + 8..0x28 + 12].copy_from_slice(&0x100u32.to_le_bytes());
        assert!(matches!(
            Ucfx::parse(&a),
            Err(UcfxError::SectionOutOfBounds { index: 1, .. })
        ));
    }

    #[test]
    fn rejects_runaway_nesting() {
        let mut node = leaf(b"leaf", b"x");
        for _ in 0..MAX_DEPTH {
            node = UcfxNode::Container(*b"nest", vec![node]);
        }
        assert_eq!(Ucfx::parse(&ucfx(&[node])).unwrap_err(), UcfxError::TooDeep);
    }

    #[test]
    fn byte_flips_never_panic() {
        let good = ucfx(&[
            leaf(b"NAME", b"n\0"),
            UcfxNode::Container(*b"COMP", vec![leaf(b"info", b"ii"), leaf(b"data", b"d")]),
        ]);
        for at in 0..good.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut a = good.clone();
                a[at] = v;
                let _ = Ucfx::parse(&a).map(|u| u.name());
            }
        }
    }
}
