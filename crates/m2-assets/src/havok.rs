//! The binary object packfiles that skeletal animation clips are stored in.
//!
//! Every clip's `data` section holds one complete packfile whose header
//! names the writer as `Havok-5.5.0-r1`. The objects are stored unfixed:
//! pointer fields are zero and are resolved through each section's fixup
//! tables instead.
//!
//! ```text
//! header, 0x40 bytes:
//! 0x00  u32 x 2   magic 0x57E0E057, 0x10C0C010
//! 0x08  u32       unknown (user tag)
//! 0x0C  u32       file version
//! 0x10  u8 x 4    layout: pointer size (4), little endian (1), two flags
//! 0x14  u32       section count
//! 0x18  u32       section holding the contents object
//! 0x1C  u32       offset of the contents object in that section
//! 0x20  u32       section holding the contents class name
//! 0x24  u32       offset of the contents class name in that section
//! 0x28  char[16]  writer version, NUL-padded
//! 0x38  u32 x 2   unknown (flags, padding)
//!
//! section header, 0x30 bytes each, after the file header:
//! 0x00  char[20]  tag, NUL-padded ("__classnames__", "__types__", "__data__")
//! 0x14  u32       start of the section in the file
//! 0x18  u32 x 6   offsets from the start: local fixups, global fixups,
//!                 virtual fixups, exports, imports, end
//! ```
//!
//! A section's object data runs from its start to its local fixups. The
//! fixup tables are padded with `0xFFFFFFFF` entries:
//!
//! ```text
//! local    {u32 pointer field, u32 target}            within the section
//! global   {u32 pointer field, u32 section, u32 target}
//! virtual  {u32 object, u32 class name section, u32 class name offset}
//! ```
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 4,232 packfiles in the animation
//! assets of a legally installed copy, all of which parse with every fixup
//! landing inside its section. No SDK code or headers were used.

use crate::bytes::{OutOfBounds, Reader};

/// The two magic words at the start of a packfile.
pub const MAGIC: [u32; 2] = [0x57E0_E057, 0x10C0_C010];
/// Length of the file header.
pub const HEADER_LEN: usize = 0x40;
/// Length of one section header.
pub const SECTION_HEADER_LEN: usize = 0x30;

const PADDING: u32 = 0xFFFF_FFFF;

/// Errors from parsing a packfile.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum PackfileError {
    #[error("not a packfile (magic {0:08x?})")]
    BadMagic([u32; 2]),
    #[error("packfile truncated: needed {len} bytes at offset {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("unsupported layout {0:02x?}; only 4-byte little-endian pointers are handled")]
    Layout([u8; 4]),
    #[error("section {section}'s tables are out of order or run past the file")]
    SectionBounds { section: usize },
    #[error("section {section} has a fixup at {offset:#x} outside its data or the file")]
    Fixup { section: usize, offset: u32 },
    #[error("no section {0}")]
    NoSection(u32),
    #[error("class name at section {section} offset {offset:#x} is not a string")]
    ClassName { section: u32, offset: u32 },
}

impl From<OutOfBounds> for PackfileError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// The file header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackfileHeader {
    pub unk_0x08: u32,
    pub file_version: u32,
    pub layout: [u8; 4],
    pub section_count: u32,
    pub contents_section: u32,
    pub contents_offset: u32,
    pub contents_class_section: u32,
    pub contents_class_offset: u32,
    /// The writer version, such as `Havok-5.5.0-r1`.
    pub version: String,
    pub unk_0x38: [u32; 2],
}

/// A pointer to another section, from a global fixup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalFixup {
    pub field: u32,
    pub section: u32,
    pub target: u32,
}

/// An object and its class, from a virtual fixup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualFixup {
    pub object: u32,
    pub class_section: u32,
    pub class_offset: u32,
}

/// One section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackSection<'a> {
    pub tag: String,
    /// Object data: from the section's start to its local fixups.
    pub data: &'a [u8],
    /// `(pointer field, target)` pairs, both offsets into `data`.
    pub local_fixups: Vec<(u32, u32)>,
    pub global_fixups: Vec<GlobalFixup>,
    pub virtual_fixups: Vec<VirtualFixup>,
}

impl PackSection<'_> {
    /// The target of the pointer stored at `field` within this section.
    pub fn local(&self, field: u32) -> Option<u32> {
        self.local_fixups.iter().find(|f| f.0 == field).map(|f| f.1)
    }

    /// The `(section, target)` of the pointer stored at `field`.
    pub fn global(&self, field: u32) -> Option<(u32, u32)> {
        self.global_fixups
            .iter()
            .find(|f| f.field == field)
            .map(|f| (f.section, f.target))
    }
}

/// A parsed packfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packfile<'a> {
    pub header: PackfileHeader,
    pub sections: Vec<PackSection<'a>>,
}

impl<'a> Packfile<'a> {
    /// Parses the headers and fixup tables, checking every fixup lands
    /// inside its section's data.
    pub fn parse(file: &'a [u8]) -> Result<Self, PackfileError> {
        let mut r = Reader::at(file, 0);
        let magic = [r.u32()?, r.u32()?];
        if magic != MAGIC {
            return Err(PackfileError::BadMagic(magic));
        }
        let unk_0x08 = r.u32()?;
        let file_version = r.u32()?;
        let layout = r.array::<4>()?;
        if layout[0] != 4 || layout[1] != 1 {
            return Err(PackfileError::Layout(layout));
        }
        let section_count = r.u32()?;
        let contents_section = r.u32()?;
        let contents_offset = r.u32()?;
        let contents_class_section = r.u32()?;
        let contents_class_offset = r.u32()?;
        let version = padded_str(r.take(16)?);
        let unk_0x38 = [r.u32()?, r.u32()?];
        let header = PackfileHeader {
            unk_0x08,
            file_version,
            layout,
            section_count,
            contents_section,
            contents_offset,
            contents_class_section,
            contents_class_offset,
            version,
            unk_0x38,
        };

        // Each header is read before it is kept, so a huge count fails on
        // the first missing header rather than allocating.
        let mut sections = Vec::new();
        for section in 0..section_count as usize {
            let tag = padded_str(r.take(20)?);
            let start = r.u32()? as usize;
            let mut offs = [0u32; 6];
            for o in &mut offs {
                *o = r.u32()?;
            }
            let bad = PackfileError::SectionBounds { section };
            if offs.windows(2).any(|w| w[0] > w[1]) {
                return Err(bad);
            }
            let end = start.checked_add(offs[5] as usize).ok_or(bad.clone())?;
            let body = file.get(start..end).ok_or(bad)?;
            let table = |i: usize| &body[offs[i] as usize..offs[i + 1] as usize];
            let data = &body[..offs[0] as usize];
            let fixup = |offset: u32| PackfileError::Fixup { section, offset };

            let mut local_fixups = Vec::new();
            for c in table(0).chunks_exact(8) {
                let [src, dst] = words(c);
                if src == PADDING {
                    continue;
                }
                for v in [src, dst] {
                    if !fits(data, v, 4 * u32::from(v == src)) {
                        return Err(fixup(v));
                    }
                }
                local_fixups.push((src, dst));
            }
            let mut global_fixups = Vec::new();
            for c in table(1).chunks_exact(12) {
                let [field, section, target] = words(c);
                if field == PADDING {
                    continue;
                }
                if !fits(data, field, 4) {
                    return Err(fixup(field));
                }
                global_fixups.push(GlobalFixup {
                    field,
                    section,
                    target,
                });
            }
            let mut virtual_fixups = Vec::new();
            for c in table(2).chunks_exact(12) {
                let [object, class_section, class_offset] = words(c);
                if object == PADDING {
                    continue;
                }
                if !fits(data, object, 0) {
                    return Err(fixup(object));
                }
                virtual_fixups.push(VirtualFixup {
                    object,
                    class_section,
                    class_offset,
                });
            }
            sections.push(PackSection {
                tag,
                data,
                local_fixups,
                global_fixups,
                virtual_fixups,
            });
        }
        let pack = Self { header, sections };
        // Cross-section targets are checked once every section is known.
        for (i, s) in pack.sections.iter().enumerate() {
            for g in &s.global_fixups {
                if !pack
                    .section(g.section)
                    .is_ok_and(|t| fits(t.data, g.target, 0))
                {
                    return Err(PackfileError::Fixup {
                        section: i,
                        offset: g.field,
                    });
                }
            }
        }
        Ok(pack)
    }

    /// Section `index`.
    pub fn section(&self, index: u32) -> Result<&PackSection<'a>, PackfileError> {
        self.sections
            .get(index as usize)
            .ok_or(PackfileError::NoSection(index))
    }

    /// The NUL-terminated class name at `offset` in section `section`.
    pub fn class_name(&self, section: u32, offset: u32) -> Result<&'a str, PackfileError> {
        let bad = PackfileError::ClassName { section, offset };
        let data = self.section(section)?.data;
        let s = Reader::at(data, offset as usize)
            .cstr()
            .map_err(|_| bad.clone())?;
        std::str::from_utf8(s).map_err(|_| bad)
    }

    /// The class of the object at `offset` in section `section`, from the
    /// virtual fixups.
    pub fn class_of(&self, section: u32, offset: u32) -> Result<Option<&'a str>, PackfileError> {
        let s = self.section(section)?;
        match s.virtual_fixups.iter().find(|v| v.object == offset) {
            Some(v) => self.class_name(v.class_section, v.class_offset).map(Some),
            None => Ok(None),
        }
    }
}

fn padded_str(bytes: &[u8]) -> String {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..len]).into_owned()
}

fn words<const N: usize>(c: &[u8]) -> [u32; N] {
    let mut out = [0; N];
    for (o, w) in out.iter_mut().zip(c.chunks_exact(4)) {
        *o = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
    }
    out
}

/// Whether `len` bytes at `offset` lie inside `data`; a zero `len` allows
/// an offset at the very end (an empty array).
fn fits(data: &[u8], offset: u32, len: u32) -> bool {
    u64::from(offset) + u64::from(len) <= data.len() as u64
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A packfile with a class name section and one data section.
    pub struct PackBuilder {
        pub classes: Vec<u8>,
        pub data: Vec<u8>,
        pub local: Vec<(u32, u32)>,
        pub global: Vec<(u32, u32, u32)>,
        pub virt: Vec<(u32, u32)>,
        pub contents: u32,
    }

    impl PackBuilder {
        pub fn new() -> Self {
            Self {
                classes: Vec::new(),
                data: Vec::new(),
                local: Vec::new(),
                global: Vec::new(),
                virt: Vec::new(),
                contents: 0,
            }
        }

        /// Adds a class name and returns its offset.
        pub fn class(&mut self, name: &str) -> u32 {
            // Each name follows a 4-byte signature and a 0x09 byte.
            self.classes
                .extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0x09]);
            let off = self.classes.len() as u32;
            self.classes.extend_from_slice(name.as_bytes());
            self.classes.push(0);
            off
        }

        /// Places an object of class `name` at `offset` in the data section.
        pub fn object(&mut self, offset: u32, name: &str) {
            let c = self.class(name);
            self.virt.push((offset, c));
        }

        pub fn build(&self) -> Vec<u8> {
            fn pad16(v: &mut Vec<u8>, fill: u8) {
                v.resize(v.len().next_multiple_of(16), fill);
            }
            let mut file = Vec::new();
            for w in MAGIC.into_iter().chain([0, 5]) {
                file.extend_from_slice(&w.to_le_bytes());
            }
            file.extend_from_slice(&[4, 1, 0, 1]);
            let contents_class = self
                .virt
                .iter()
                .find(|v| v.0 == self.contents)
                .map_or(0, |v| v.1);
            for w in [3, 2, self.contents, 0, contents_class] {
                file.extend_from_slice(&w.to_le_bytes());
            }
            let mut ver = b"Havok-5.5.0-r1".to_vec();
            ver.resize(16, 0);
            file.extend_from_slice(&ver);
            file.extend_from_slice(&[0xFF; 8]);

            // Section bodies, each data then local, global, virtual tables.
            let mut classes = self.classes.clone();
            pad16(&mut classes, 0xFF);
            let mut data = self.data.clone();
            pad16(&mut data, 0);
            let mut local = Vec::new();
            for &(a, b) in &self.local {
                local.extend([a, b].iter().flat_map(|w| w.to_le_bytes()));
            }
            pad16(&mut local, 0xFF);
            let mut global = Vec::new();
            for &(a, b, c) in &self.global {
                global.extend([a, b, c].iter().flat_map(|w| w.to_le_bytes()));
            }
            pad16(&mut global, 0xFF);
            let mut virt = Vec::new();
            for &(a, c) in &self.virt {
                virt.extend([a, 0, c].iter().flat_map(|w| w.to_le_bytes()));
            }
            pad16(&mut virt, 0xFF);

            let mut start = (HEADER_LEN + 3 * SECTION_HEADER_LEN) as u32;
            let mut bodies = Vec::new();
            let sections: [(&str, Vec<u8>, [usize; 3]); 3] = [
                ("__classnames__", classes, [0, 0, 0]),
                ("__types__", Vec::new(), [0, 0, 0]),
                ("__data__", data, [local.len(), global.len(), virt.len()]),
            ];
            for (tag, body, [l, g, v]) in &sections {
                let mut t = tag.as_bytes().to_vec();
                t.resize(20, 0);
                file.extend_from_slice(&t);
                let d = body.len() as u32;
                let (l, g, v) = (*l as u32, *g as u32, *v as u32);
                let end = d + l + g + v;
                for w in [start, d, d + l, d + l + g, end, end, end] {
                    file.extend_from_slice(&w.to_le_bytes());
                }
                start += end;
                bodies.push(body.clone());
            }
            file.extend_from_slice(&bodies[0]);
            file.extend_from_slice(&bodies[2]);
            file.extend_from_slice(&local);
            file.extend_from_slice(&global);
            file.extend_from_slice(&virt);
            file
        }
    }

    fn sample() -> PackBuilder {
        let mut b = PackBuilder::new();
        b.data = vec![0; 0x20];
        b.object(0, "Container");
        b.object(0x10, "Thing");
        b.local.push((4, 0x0C));
        b.global.push((0x0C, 2, 0x10));
        b
    }

    #[test]
    fn parses_sections_and_fixups() {
        let file = sample().build();
        let p = Packfile::parse(&file).unwrap();
        assert_eq!(p.header.version, "Havok-5.5.0-r1");
        assert_eq!(p.header.contents_section, 2);
        assert_eq!(p.sections.len(), 3);
        assert_eq!(p.sections[0].tag, "__classnames__");
        let data = &p.sections[2];
        assert_eq!(data.data.len(), 0x20);
        assert_eq!(data.local(4), Some(0x0C));
        assert_eq!(data.local(8), None);
        assert_eq!(data.global(0x0C), Some((2, 0x10)));
        assert_eq!(p.class_of(2, 0).unwrap(), Some("Container"));
        assert_eq!(p.class_of(2, 0x10).unwrap(), Some("Thing"));
        assert_eq!(p.class_of(2, 4).unwrap(), None);
        let h = &p.header;
        assert_eq!(
            p.class_name(h.contents_class_section, h.contents_class_offset),
            Ok("Container")
        );
    }

    #[test]
    fn rejects_bad_magic_and_layout() {
        let mut file = sample().build();
        file[0] ^= 1;
        assert!(matches!(
            Packfile::parse(&file),
            Err(PackfileError::BadMagic(_))
        ));
        let mut file = sample().build();
        file[0x10] = 8;
        assert!(matches!(
            Packfile::parse(&file),
            Err(PackfileError::Layout(_))
        ));
    }

    #[test]
    fn rejects_fixups_outside_the_section() {
        let mut b = sample();
        b.local.push((0x1E, 0));
        assert_eq!(
            Packfile::parse(&b.build()),
            Err(PackfileError::Fixup {
                section: 2,
                offset: 0x1E
            })
        );
        let mut b = sample();
        b.global.push((0, 7, 0));
        assert!(matches!(
            Packfile::parse(&b.build()),
            Err(PackfileError::Fixup { .. })
        ));
    }

    #[test]
    fn truncated_files_fail_cleanly() {
        let file = sample().build();
        for len in 0..file.len() {
            assert!(Packfile::parse(&file[..len]).is_err(), "length {len}");
        }
    }
}
