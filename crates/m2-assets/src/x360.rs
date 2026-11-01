//! The Xbox 360 build's archives, read-only, as a research reference for
//! the PC formats (the user's own 360 disc, extracted locally; nothing from
//! it is committed).
//!
//! From samples (disc 1's `vz.wad`): the containers are the PC's
//! ([`crate::wad`], [`crate::sges`], [`crate::block`], [`crate::ucfx`])
//! written big-endian:
//! - the archive magic reads `SCFF` and its chunk tags `XDNI`, `ATAD`,
//!   `MUSC`, `TESA`, `SHTP` (each 4-byte word byte-swapped); a block record's
//!   two size words are big-endian `u32`s whose low half is the sector
//!   count and high half the flags, as on the PC; an asset record's type
//!   index is the one field stored as on the PC (byte 12, then three zero
//!   bytes);
//! - `sges` keeps its magic and swaps its fields; segments are raw DEFLATE,
//!   as on the PC;
//! - a block's directory and a `UCFX` container swap every word (magic
//!   `XFCU`, each section's fourcc reversed), and so do the values inside
//!   (`ATRB`: name hash, a word of kind in its low byte and flags in the
//!   next, a big-endian value; `AKEY`: big-endian time and value);
//! - as on the PC ([`crate::ucfx::Section::own`]), an `EFCT`, `EMTR` or
//!   `FRCE` section carries data of its own as well as its children, and
//!   here so does a keyed `ATRB`. A
//!   `FRCE`'s data starts with its kind's tag ([`ForceKind::tag`]) and then
//!   the kind's own settings, as the 360 effect reader takes them (from
//!   the 360 code, 0x821E0C00).
//!
//! The vz archive holds 11,087 blocks, 30,553 assets and 36 types; its 314
//! effects sit in one block (`blocks\vz\effects_P000_Q3.block`), as the
//! PC's 314 do.

use std::io::{self, Read, Seek, SeekFrom};

use crate::effect::ForceKind;
use crate::wad::{Asset, Block, Header, SECTOR_SIZE};

/// The archive magic as the 360 stores it.
pub const MAGIC: [u8; 4] = *b"SCFF";
/// The `UCFX` magic as the 360 stores it.
pub const UCFX_MAGIC: [u8; 4] = *b"XFCU";
/// The `sges` magic as the 360 stores it.
pub const SGES_MAGIC: [u8; 4] = *b"segs";
const CHUNKS: [[u8; 4]; 5] = [*b"XDNI", *b"ATAD", *b"MUSC", *b"TESA", *b"SHTP"];
/// Largest header-and-tables region read (the archive uses about 2 MiB).
const MAX_TABLES_LEN: u32 = 64 << 20;

/// Errors from reading a 360 archive.
#[derive(Debug, thiserror::Error)]
pub enum X360Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("not a 360 archive (magic {0:02x?})")]
    BadMagic([u8; 4]),
    #[error("chunk record {0} has the wrong tag")]
    BadChunk(usize),
    #[error("{0} truncated")]
    Truncated(&'static str),
    #[error("no such block {0}")]
    NoSuchBlock(usize),
    #[error("not a 360 UCFX container")]
    BadUcfx,
    #[error("segment {0} failed to inflate")]
    Inflate(usize),
}

/// Big-endian reads from a slice.
struct Be<'a> {
    buf: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Be<'a> {
    fn at(buf: &'a [u8], pos: usize, what: &'static str) -> Self {
        Self { buf, pos, what }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], X360Error> {
        let s = self
            .pos
            .checked_add(n)
            .and_then(|end| self.buf.get(self.pos..end))
            .ok_or(X360Error::Truncated(self.what))?;
        self.pos += n;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], X360Error> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u16(&mut self) -> Result<u16, X360Error> {
        self.array().map(u16::from_be_bytes)
    }

    fn u32(&mut self) -> Result<u32, X360Error> {
        self.array().map(u32::from_be_bytes)
    }
}

/// An open 360 archive: its header and tables, in the PC's types.
#[derive(Debug)]
pub struct Archive<R> {
    pub header: Header,
    pub type_hashes: Vec<u32>,
    pub blocks: Vec<Block>,
    pub assets: Vec<Asset>,
    reader: R,
}

impl<R: Read + Seek> Archive<R> {
    /// Reads the header and tables.
    pub fn open(mut reader: R) -> Result<Self, X360Error> {
        reader.seek(SeekFrom::Start(0))?;
        let mut head = [0; 0x48];
        reader.read_exact(&mut head)?;
        let header = parse_header(&head)?;
        if header.data_offset > MAX_TABLES_LEN || header.data_offset < 0x48 {
            return Err(X360Error::Truncated("tables"));
        }
        let mut t = head.to_vec();
        t.resize(header.data_offset as usize, 0);
        reader.read_exact(&mut t[0x48..])?;
        let (type_hashes, blocks, assets) = parse_tables(&t, &header)?;
        Ok(Self {
            header,
            type_hashes,
            blocks,
            assets,
            reader,
        })
    }

    /// Reads and decodes block `index`.
    pub fn read_block(&mut self, index: usize) -> Result<Vec<u8>, X360Error> {
        let b = self
            .blocks
            .get(index)
            .ok_or(X360Error::NoSuchBlock(index))?;
        let mut raw = vec![0; usize::from(b.stored_sectors) * SECTOR_SIZE as usize];
        self.reader.seek(SeekFrom::Start(b.offset()))?;
        self.reader.read_exact(&mut raw)?;
        decode_sges(&raw)
    }

    /// The type hash of `asset`.
    pub fn type_hash(&self, asset: &Asset) -> Option<u32> {
        self.type_hashes.get(asset.type_index as usize).copied()
    }
}

fn parse_header(h: &[u8]) -> Result<Header, X360Error> {
    let mut r = Be::at(h, 0, "header");
    let magic = r.array::<4>()?;
    if magic != MAGIC {
        return Err(X360Error::BadMagic(magic));
    }
    let version = r.u32()?;
    let unk_0x08 = r.u32()?;
    let mut c = [(0, 0); 5];
    for (i, tag) in CHUNKS.iter().enumerate() {
        if r.array::<4>()? != *tag {
            return Err(X360Error::BadChunk(i));
        }
        c[i] = (r.u32()?, r.u32()?);
    }
    Ok(Header {
        version,
        unk_0x08,
        index_offset: c[0].0,
        block_count: c[0].1,
        data_offset: c[1].0,
        type_count: c[1].1,
        checksum: c[2].0,
        checksum_count: c[2].1,
        asset_offset: c[3].0,
        asset_count: c[3].1,
        paths_offset: c[4].0,
        path_count: c[4].1,
    })
}

type Tables = (Vec<u32>, Vec<Block>, Vec<Asset>);

fn parse_tables(t: &[u8], h: &Header) -> Result<Tables, X360Error> {
    let mut r = Be::at(t, 0x48, "type table");
    let types = (0..h.type_count)
        .map(|_| r.u32())
        .collect::<Result<Vec<_>, _>>()?;
    let mut r = Be::at(t, h.index_offset as usize, "block table");
    let mut blocks = Vec::with_capacity(h.block_count as usize);
    for _ in 0..h.block_count {
        let start_sector = r.u32()?;
        let decoded = r.u32()?;
        let stored = r.u32()?;
        blocks.push(Block {
            path: String::new(),
            start_sector,
            sectors: decoded as u16,
            unk_sectors_hi: (decoded >> 16) as u16,
            stored_sectors: stored as u16,
            flags: (stored >> 16) as u16,
        });
    }
    let mut r = Be::at(t, h.asset_offset as usize, "asset table");
    let mut assets = Vec::with_capacity(h.asset_count as usize);
    for _ in 0..h.asset_count {
        let name_hash = r.u32()?;
        let alt_blocks = [r.u16()?, r.u16()?, r.u16()?];
        let block = r.u16()?;
        // Stored as on the PC: little-endian.
        let type_index = u32::from_le_bytes(r.array()?);
        assets.push(Asset {
            name_hash,
            alt_blocks,
            block,
            type_index,
        });
    }
    let mut at = h.paths_offset as usize;
    for b in &mut blocks {
        let rest = t.get(at..).ok_or(X360Error::Truncated("path table"))?;
        let len = rest
            .iter()
            .position(|&c| c == 0)
            .ok_or(X360Error::Truncated("path table"))?;
        b.path = String::from_utf8_lossy(&rest[..len]).into_owned();
        at += len + 1;
    }
    Ok((types, blocks, assets))
}

/// Decodes a big-endian `sges` container (see [`crate::sges`]).
pub fn decode_sges(raw: &[u8]) -> Result<Vec<u8>, X360Error> {
    let mut r = Be::at(raw, 0, "sges");
    if r.array::<4>()? != SGES_MAGIC {
        return Err(X360Error::Truncated("sges magic"));
    }
    let _version = r.u16()?;
    let count = r.u16()?;
    let total = r.u32()?;
    let _stored = r.u32()?;
    let mut out = Vec::with_capacity(total as usize);
    for i in 0..usize::from(count) {
        let stored = match r.u16()? {
            0 => 0x1_0000,
            n => usize::from(n),
        };
        let decoded = match r.u16()? {
            0 => 0x1_0000,
            n => usize::from(n),
        };
        let offset = r.u32()?;
        let at = (offset & !1) as usize;
        let data = raw
            .get(at..at + stored)
            .ok_or(X360Error::Truncated("sges segment"))?;
        if offset & 1 == 1 {
            let d = miniz_oxide::inflate::decompress_to_vec_with_limit(data, decoded)
                .map_err(|_| X360Error::Inflate(i))?;
            out.extend_from_slice(&d);
        } else {
            out.extend_from_slice(data);
        }
    }
    Ok(out)
}

/// One asset in a decoded block: name and type hashes and its bytes.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub name_hash: u32,
    pub type_hash: u32,
    pub data: &'a [u8],
}

/// Splits a decoded block by its big-endian directory (see
/// [`crate::block`]).
pub fn block_entries(block: &[u8]) -> Result<Vec<Entry<'_>>, X360Error> {
    let mut r = Be::at(block, 0, "block directory");
    let count = r.u32()? as usize;
    let mut at = 4 + 16 * count;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let name_hash = r.u32()?;
        let type_hash = r.u32()?;
        let _unk = r.u32()?;
        let len = r.u32()? as usize;
        let data = block
            .get(at..at + len)
            .ok_or(X360Error::Truncated("block asset"))?;
        at += len;
        out.push(Entry {
            name_hash,
            type_hash,
            data,
        });
    }
    Ok(out)
}

/// One section of a big-endian `UCFX` container, in depth-first order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section<'a> {
    /// In the PC's order (`FRCE`, not `ECRF`).
    pub fourcc: [u8; 4],
    pub depth: usize,
    pub descendants: u32,
    /// Its own data, `None` where it has none.
    pub data: Option<&'a [u8]>,
}

/// Parses a big-endian `UCFX` container's sections.
pub fn sections(asset: &[u8]) -> Result<Vec<Section<'_>>, X360Error> {
    let mut r = Be::at(asset, 0, "UCFX");
    if r.array::<4>()? != UCFX_MAGIC {
        return Err(X360Error::BadUcfx);
    }
    let header_len = r.u32()? as usize;
    r.u32()?;
    r.u32()?;
    let count = r.u32()? as usize;
    if header_len != 0x14 + 20 * count {
        return Err(X360Error::BadUcfx);
    }
    let payload = asset.get(header_len..).ok_or(X360Error::BadUcfx)?;
    let mut out = Vec::with_capacity(count);
    // Index one past each open container's subtree.
    let mut open: Vec<usize> = Vec::new();
    for index in 0..count {
        let mut fourcc = r.array::<4>()?;
        fourcc.reverse();
        let offset = r.u32()?;
        let len = r.u32()? as usize;
        let _siblings = r.u32()?;
        let descendants = r.u32()?;
        while open.last() == Some(&index) {
            open.pop();
        }
        let data = if offset == u32::MAX {
            None
        } else {
            let at = offset as usize;
            Some(payload.get(at..at + len).ok_or(X360Error::BadUcfx)?)
        };
        out.push(Section {
            fourcc,
            depth: open.len(),
            descendants,
            data,
        });
        if descendants > 0 {
            open.push(index + 1 + descendants as usize);
        }
    }
    Ok(out)
}

/// One `ATRB` as the 360 stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub name: u32,
    pub kind: u8,
    pub flags: u8,
    /// The value's bits, big-endian decoded.
    pub value: u32,
    /// Whether it has keys under it.
    pub keyed: bool,
    /// Its `AKEY`s, `(time, value)`.
    pub keys: Vec<(f32, f32)>,
}

impl Setting {
    pub fn float(&self) -> f32 {
        f32::from_bits(self.value)
    }
}

/// One `FRCE` as the 360 stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Force {
    /// The tag its data starts with.
    pub tag: u32,
    /// The kind the tag names, `None` for an unknown tag.
    pub kind: Option<ForceKind>,
    /// The kind's own settings after the tag, as big-endian words.
    pub params: Vec<u32>,
    /// Its `ATRB`s, in order.
    pub settings: Vec<Setting>,
}

/// An effect asset's forces, in order.
pub fn effect_forces(asset: &[u8]) -> Result<Vec<Force>, X360Error> {
    let s = sections(asset)?;
    let mut out = Vec::new();
    for (i, f) in s.iter().enumerate() {
        if &f.fourcc != b"FRCE" {
            continue;
        }
        let data = f.data.unwrap_or_default();
        let words: Vec<u32> = data
            .chunks_exact(4)
            .map(|w| u32::from_be_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let tag = words.first().copied().unwrap_or(0);
        let end = i + 1 + f.descendants as usize;
        let under = &s[i + 1..end.min(s.len())];
        let settings = under
            .iter()
            .enumerate()
            .filter(|(_, c)| c.depth == f.depth + 1 && &c.fourcc == b"ATRB")
            .filter_map(|(j, c)| {
                let d = c.data?;
                let w = |k: usize| u32::from_be_bytes([d[k], d[k + 1], d[k + 2], d[k + 3]]);
                (d.len() >= 12).then(|| Setting {
                    name: w(0),
                    kind: w(4) as u8,
                    flags: (w(4) >> 8) as u8,
                    value: w(8),
                    keyed: c.descendants > 0,
                    keys: keys(&under[j + 1..(j + 1 + c.descendants as usize).min(under.len())]),
                })
            })
            .collect();
        out.push(Force {
            tag,
            kind: ForceKind::from_tag(tag),
            params: words.get(1..).unwrap_or_default().to_vec(),
            settings,
        });
    }
    Ok(out)
}

/// The `AKEY`s among `under`, as `(time, value)`.
fn keys(under: &[Section<'_>]) -> Vec<(f32, f32)> {
    under
        .iter()
        .filter(|c| &c.fourcc == b"AKEY")
        .filter_map(|c| {
            let d = c.data?;
            let f = |k: usize| f32::from_be_bytes([d[k], d[k + 1], d[k + 2], d[k + 3]]);
            (d.len() >= 8).then(|| (f(0), f(4)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// A record: fourcc as the 360 stores it (reversed), offset, length,
    /// siblings after, descendants.
    fn rec(cc: &[u8; 4], off: u32, len: u32, sib: u32, sub: u32) -> Vec<u8> {
        let mut r: Vec<u8> = cc.iter().rev().copied().collect();
        r.extend(be(&[off, len, sib, sub]));
        r
    }

    /// An effect with one vortex force: a tag and one param word, and two
    /// settings under it, the second keyed.
    fn effect() -> Vec<u8> {
        let recs = [
            rec(b"EFCT", u32::MAX, 0, 0, 5),
            rec(b"FRCE", 0, 8, 0, 4),
            rec(b"ATRB", 8, 12, 1, 0),
            rec(b"ATRB", 20, 12, 0, 2),
            rec(b"ANIM", 32, 4, 0, 1),
            rec(b"AKEY", 36, 8, 0, 0),
        ];
        let mut a = b"XFCU".to_vec();
        a.extend(be(&[
            0x14 + 20 * recs.len() as u32,
            0,
            0,
            recs.len() as u32,
        ]));
        for r in &recs {
            a.extend(r);
        }
        a.extend(be(&[0xf4d8_5a49, 0x3f80_0000]));
        a.extend(be(&[0x3dc3_d9df, 0x0000_0001, 0x4040_0000]));
        a.extend(be(&[0xe068_6a68, 0x0000_0701, 0x40a0_0000]));
        a.extend(be(&[1, 0, 0x4000_0000]));
        a
    }

    #[test]
    fn reads_a_force_s_tag_params_and_settings() {
        let a = effect();
        let s = sections(&a).unwrap();
        assert_eq!(s.len(), 6);
        assert_eq!(&s[1].fourcc, b"FRCE");
        assert_eq!((s[1].depth, s[2].depth, s[5].depth), (1, 2, 4));
        let f = effect_forces(&a).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, Some(ForceKind::Vortex));
        assert_eq!(f[0].params, vec![0x3f80_0000]);
        let names: Vec<u32> = f[0].settings.iter().map(|s| s.name).collect();
        assert_eq!(names, vec![0x3dc3_d9df, 0xe068_6a68]);
        assert_eq!(f[0].settings[0].float(), 3.0);
        assert_eq!((f[0].settings[1].kind, f[0].settings[1].flags), (1, 7));
        assert!(f[0].settings[1].keyed && !f[0].settings[0].keyed);
        assert_eq!(f[0].settings[1].keys, vec![(0.0, 2.0)]);
    }

    #[test]
    fn decodes_a_big_endian_sges_and_block_directory() {
        // One stored segment holding a directory of one 4-byte asset.
        let body = [be(&[1, 0xaaaa_0001, 0xbbbb_0002, 0, 4]), vec![9, 8, 7, 6]].concat();
        let mut raw = b"segs".to_vec();
        raw.extend(4u16.to_be_bytes());
        raw.extend(1u16.to_be_bytes());
        raw.extend(be(&[body.len() as u32, body.len() as u32]));
        raw.extend((body.len() as u16).to_be_bytes());
        raw.extend((body.len() as u16).to_be_bytes());
        raw.extend(be(&[0x18]));
        raw.extend(&body);
        let block = decode_sges(&raw).unwrap();
        assert_eq!(block, body);
        let e = block_entries(&block).unwrap();
        assert_eq!((e[0].name_hash, e[0].type_hash), (0xaaaa_0001, 0xbbbb_0002));
        assert_eq!(e[0].data, &[9, 8, 7, 6]);
    }

    #[test]
    fn reads_the_header_and_rejects_a_pc_archive() {
        let mut h = b"SCFF".to_vec();
        h.extend(be(&[2, 7]));
        for (i, tag) in CHUNKS.iter().enumerate() {
            h.extend(tag);
            h.extend(be(&[i as u32 * 16, i as u32 + 1]));
        }
        let header = parse_header(&h).unwrap();
        assert_eq!((header.version, header.block_count), (2, 1));
        assert_eq!((header.asset_offset, header.path_count), (48, 5));
        h[..4].copy_from_slice(b"FFCS");
        assert!(matches!(parse_header(&h), Err(X360Error::BadMagic(_))));
    }
}
