//! The `FFCS` archive container (`data\*.wad`).
//!
//! An archive is a header, three tables, then blocks of asset data. All
//! integers are little-endian. Blocks are placed on 32 KiB sectors and the
//! file is padded to a whole number of sectors; the largest archive is over
//! 2 GiB, so file offsets are `u64`.
//!
//! ```text
//! 0x00  [u8; 4]  magic "FFCS"
//! 0x04  u32      version (2)
//! 0x08  u32      unknown (7 in every archive seen)
//! 0x0C  five chunk records { [u8; 4] fourcc, u32 a, u32 b }, in this order:
//!       INDX  a = offset of the block index    b = block count
//!       DATA  a = offset of the first block    b = type count
//!       CSUM  a = 32-bit checksum or hash      b = unknown count
//!       ASET  a = offset of the asset table    b = asset count
//!       PTHS  a = offset of the path table     b = path count (= block count)
//! 0x48  [u32; type count]  type-name hashes; assets refer to them by index
//!
//! INDX record, 12 bytes:
//!       u32  first sector
//!       u16  decoded size in sectors, u16 unknown
//!       u16  stored size in sectors,  u16 flags (unknown; top byte 0x80..)
//! ASET record, 16 bytes:
//!       u32       asset name hash
//!       [u16; 3]  further block indices, 0xFFFF = none (meaning unknown)
//!       u16       block index
//!       u32       type index
//! PTHS: one NUL-terminated ASCII path per block, in block order,
//!       e.g. `blocks\Loading\loading_base_P000_Q3.block`.
//! ```
//!
//! Each block's stored bytes are an [`sges`] container. The hash
//! function behind the name and type hashes is not yet known, so assets are
//! found by hash and blocks by path.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};

use crate::bytes::{OutOfBounds, Reader};
use crate::sges::{self, SgesError};

/// Archive magic.
pub const MAGIC: [u8; 4] = *b"FFCS";
/// The only archive version seen.
pub const VERSION: u32 = 2;
/// Block placement unit.
pub const SECTOR_SIZE: u64 = 0x8000;
/// Largest header-and-tables region this parser will read. The real
/// archives use 2 MiB.
pub const MAX_TABLES_LEN: u64 = 64 << 20;
/// Marks an unused entry in [`Asset::alt_blocks`].
pub const NO_BLOCK: u16 = 0xFFFF;

const CHUNK_ORDER: [[u8; 4]; 5] = [*b"INDX", *b"DATA", *b"CSUM", *b"ASET", *b"PTHS"];
const TYPE_TABLE_OFFSET: usize = 0x48;
const BLOCK_RECORD_LEN: usize = 12;
const ASSET_RECORD_LEN: usize = 16;

/// Errors from reading an archive.
#[derive(Debug, thiserror::Error)]
pub enum WadError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("not an FFCS archive (magic {0:02x?})")]
    BadMagic([u8; 4]),
    #[error("unsupported archive version {0}")]
    UnsupportedVersion(u32),
    #[error("chunk record {index} is {found:02x?}, expected {expected:02x?}")]
    UnexpectedChunk {
        index: usize,
        expected: [u8; 4],
        found: [u8; 4],
    },
    #[error("archive tables truncated: needed {len} bytes at offset {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("{table} table at {offset:#x} runs past the data offset {data_offset:#x}")]
    TableOutOfBounds {
        table: &'static str,
        offset: u64,
        data_offset: u32,
    },
    #[error("data offset {0:#x} exceeds the {MAX_TABLES_LEN:#x}-byte table limit")]
    TablesTooLarge(u32),
    #[error("{paths} paths for {blocks} blocks")]
    PathCountMismatch { blocks: u32, paths: u32 },
    #[error("path {index} is not ASCII")]
    NonAsciiPath { index: usize },
    #[error("asset {asset} refers to block {block}, but there are {blocks} blocks")]
    BadBlockIndex {
        asset: usize,
        block: u16,
        blocks: u32,
    },
    #[error("asset {asset} has type index {index}, but there are {types} types")]
    BadTypeIndex {
        asset: usize,
        index: u32,
        types: u32,
    },
    #[error("block {block} (sectors {start:#x}+{count:#x}) lies outside the data area")]
    BlockOutOfBounds {
        block: usize,
        start: u32,
        count: u16,
    },
    #[error("block index {0} out of range")]
    NoSuchBlock(usize),
    #[error("block {block}: {source}")]
    Block {
        block: usize,
        #[source]
        source: SgesError,
    },
}

impl From<OutOfBounds> for WadError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// The fixed archive header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub unk_0x08: u32,
    pub index_offset: u32,
    pub block_count: u32,
    pub data_offset: u32,
    pub type_count: u32,
    /// CSUM `a`: a 32-bit checksum or hash; what it covers is unknown.
    pub checksum: u32,
    /// CSUM `b`: unknown.
    pub checksum_count: u32,
    pub asset_offset: u32,
    pub asset_count: u32,
    pub paths_offset: u32,
    pub path_count: u32,
}

/// Length of the fixed header before the type table.
pub const HEADER_LEN: usize = TYPE_TABLE_OFFSET;

impl Header {
    /// Parses the fixed 0x48-byte header.
    pub fn parse(buf: &[u8]) -> Result<Self, WadError> {
        let mut r = Reader::at(buf, 0);
        let magic = r.array::<4>()?;
        if magic != MAGIC {
            return Err(WadError::BadMagic(magic));
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(WadError::UnsupportedVersion(version));
        }
        let unk_0x08 = r.u32()?;
        let mut chunks = [(0u32, 0u32); 5];
        for (index, (expected, chunk)) in CHUNK_ORDER.iter().zip(&mut chunks).enumerate() {
            let found = r.array::<4>()?;
            if found != *expected {
                return Err(WadError::UnexpectedChunk {
                    index,
                    expected: *expected,
                    found,
                });
            }
            *chunk = (r.u32()?, r.u32()?);
        }
        let [indx, data, csum, aset, pths] = chunks;
        Ok(Self {
            version,
            unk_0x08,
            index_offset: indx.0,
            block_count: indx.1,
            data_offset: data.0,
            type_count: data.1,
            checksum: csum.0,
            checksum_count: csum.1,
            asset_offset: aset.0,
            asset_count: aset.1,
            paths_offset: pths.0,
            path_count: pths.1,
        })
    }
}

/// One block: a stored `sges` container holding a group of assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Path from the PTHS table, e.g. `blocks\VZ\vz_state_..._P000_Q3.block`.
    pub path: String,
    pub start_sector: u32,
    /// Decoded size rounded up to sectors.
    pub sectors: u16,
    /// High half of the decoded-size word; usually 0.
    pub unk_sectors_hi: u16,
    pub stored_sectors: u16,
    /// High half of the stored-size word; meaning unknown.
    pub flags: u16,
}

impl Block {
    /// Byte offset of the block in the archive.
    pub fn offset(&self) -> u64 {
        u64::from(self.start_sector) * SECTOR_SIZE
    }

    /// Bytes the block occupies on disk, including sector padding.
    pub fn stored_len(&self) -> u64 {
        u64::from(self.stored_sectors) * SECTOR_SIZE
    }
}

/// One asset table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    pub name_hash: u32,
    /// Extra block indices, [`NO_BLOCK`] when unused. Filled only in the
    /// largest archive; possibly quality or streaming variants.
    pub alt_blocks: [u16; 3],
    pub block: u16,
    /// Index into [`WadIndex::type_hashes`].
    pub type_index: u32,
}

impl Asset {
    /// The used entries of [`alt_blocks`](Self::alt_blocks).
    pub fn alt_blocks(&self) -> impl Iterator<Item = u16> + '_ {
        self.alt_blocks.iter().copied().filter(|&b| b != NO_BLOCK)
    }
}

/// The parsed header and tables of an archive.
#[derive(Debug, Clone)]
pub struct WadIndex {
    header: Header,
    type_hashes: Vec<u32>,
    blocks: Vec<Block>,
    assets: Vec<Asset>,
    by_path: HashMap<String, usize>,
}

/// Lookup key for a block path: ASCII lowercase with `\` separators.
fn path_key(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c == '/' {
                '\\'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

/// Checks that a table of `count` records of `record_len` bytes at `offset`
/// ends at or before `data_offset`.
fn check_table(
    table: &'static str,
    offset: u32,
    count: u32,
    record_len: usize,
    data_offset: u32,
) -> Result<usize, WadError> {
    let end = u64::from(offset) + u64::from(count) * record_len as u64;
    if end > u64::from(data_offset) {
        return Err(WadError::TableOutOfBounds {
            table,
            offset: offset.into(),
            data_offset,
        });
    }
    // Bounded by data_offset, which fits in usize.
    Ok(offset as usize)
}

impl WadIndex {
    /// Parses the header and tables from the start of an archive. `tables`
    /// must cover at least the bytes before the data offset.
    pub fn parse(tables: &[u8]) -> Result<Self, WadError> {
        let header = Header::parse(tables)?;
        if u64::from(header.data_offset) > MAX_TABLES_LEN {
            return Err(WadError::TablesTooLarge(header.data_offset));
        }
        if header.path_count != header.block_count {
            return Err(WadError::PathCountMismatch {
                blocks: header.block_count,
                paths: header.path_count,
            });
        }
        let data = header.data_offset;
        let tables = tables
            .get(..data as usize)
            .ok_or_else(|| WadError::Truncated {
                offset: tables.len(),
                len: data as usize - tables.len(),
            })?;

        let types_at = check_table("type", HEADER_LEN as u32, header.type_count, 4, data)?;
        let mut r = Reader::at(tables, types_at);
        let type_hashes = (0..header.type_count)
            .map(|_| r.u32())
            .collect::<Result<Vec<_>, _>>()?;

        let indx_at = check_table(
            "INDX",
            header.index_offset,
            header.block_count,
            BLOCK_RECORD_LEN,
            data,
        )?;
        let first_data_sector = u64::from(data).div_ceil(SECTOR_SIZE);
        let mut r = Reader::at(tables, indx_at);
        let mut blocks = Vec::with_capacity(header.block_count as usize);
        for block in 0..header.block_count as usize {
            let start_sector = r.u32()?;
            let sectors = r.u16()?;
            let unk_sectors_hi = r.u16()?;
            let stored_sectors = r.u16()?;
            let flags = r.u16()?;
            if u64::from(start_sector) < first_data_sector {
                return Err(WadError::BlockOutOfBounds {
                    block,
                    start: start_sector,
                    count: stored_sectors,
                });
            }
            blocks.push(Block {
                path: String::new(),
                start_sector,
                sectors,
                unk_sectors_hi,
                stored_sectors,
                flags,
            });
        }

        let aset_at = check_table(
            "ASET",
            header.asset_offset,
            header.asset_count,
            ASSET_RECORD_LEN,
            data,
        )?;
        let mut r = Reader::at(tables, aset_at);
        let mut assets = Vec::with_capacity(header.asset_count as usize);
        for asset in 0..header.asset_count as usize {
            let a = Asset {
                name_hash: r.u32()?,
                alt_blocks: [r.u16()?, r.u16()?, r.u16()?],
                block: r.u16()?,
                type_index: r.u32()?,
            };
            for block in std::iter::once(a.block).chain(a.alt_blocks()) {
                if u32::from(block) >= header.block_count {
                    return Err(WadError::BadBlockIndex {
                        asset,
                        block,
                        blocks: header.block_count,
                    });
                }
            }
            if a.type_index >= header.type_count {
                return Err(WadError::BadTypeIndex {
                    asset,
                    index: a.type_index,
                    types: header.type_count,
                });
            }
            assets.push(a);
        }

        // Paths are variable length; every one must end before the data.
        let pths_at = check_table("PTHS", header.paths_offset, 0, 1, data)?;
        let mut r = Reader::at(tables, pths_at);
        let mut by_path = HashMap::with_capacity(blocks.len());
        for (index, block) in blocks.iter_mut().enumerate() {
            let raw = r.cstr()?;
            if !raw.is_ascii() {
                return Err(WadError::NonAsciiPath { index });
            }
            // ASCII is valid UTF-8.
            block.path = String::from_utf8_lossy(raw).into_owned();
            by_path.entry(path_key(&block.path)).or_insert(index);
        }

        Ok(Self {
            header,
            type_hashes,
            blocks,
            assets,
            by_path,
        })
    }

    /// Reads and parses the header and tables from `reader`, which is left at
    /// an unspecified position.
    pub fn read_from<R: Read + Seek>(reader: &mut R) -> Result<Self, WadError> {
        reader.seek(SeekFrom::Start(0))?;
        let mut head = [0; HEADER_LEN];
        reader.read_exact(&mut head)?;
        let header = Header::parse(&head)?;
        if u64::from(header.data_offset) > MAX_TABLES_LEN {
            return Err(WadError::TablesTooLarge(header.data_offset));
        }
        let mut tables = head.to_vec();
        tables.resize(header.data_offset.max(HEADER_LEN as u32) as usize, 0);
        reader.read_exact(&mut tables[HEADER_LEN..])?;
        Self::parse(&tables)
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Type-name hashes; [`Asset::type_index`] indexes this.
    pub fn type_hashes(&self) -> &[u32] {
        &self.type_hashes
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }

    /// Finds a block by path, ignoring ASCII case and accepting `/` or `\`.
    pub fn find_block(&self, path: &str) -> Option<usize> {
        self.by_path.get(&path_key(path)).copied()
    }

    /// Assets whose primary block is `block`.
    pub fn assets_in_block(&self, block: usize) -> impl Iterator<Item = &Asset> + '_ {
        self.assets
            .iter()
            .filter(move |a| usize::from(a.block) == block)
    }

    /// Assets with the given name hash (several types can share a name).
    pub fn assets_by_hash(&self, name_hash: u32) -> impl Iterator<Item = &Asset> + '_ {
        self.assets.iter().filter(move |a| a.name_hash == name_hash)
    }

    /// The type-name hash of an asset.
    pub fn type_hash(&self, asset: &Asset) -> u32 {
        // Checked against type_count in `parse`.
        self.type_hashes[asset.type_index as usize]
    }

    /// Checks every block lies inside an archive of `file_len` bytes.
    pub fn check_file_len(&self, file_len: u64) -> Result<(), WadError> {
        for (block, b) in self.blocks.iter().enumerate() {
            if b.offset() + b.stored_len() > file_len {
                return Err(WadError::BlockOutOfBounds {
                    block,
                    start: b.start_sector,
                    count: b.stored_sectors,
                });
            }
        }
        Ok(())
    }
}

/// An open archive: its index plus the reader to fetch blocks from.
#[derive(Debug)]
pub struct WadArchive<R> {
    index: WadIndex,
    reader: R,
}

impl<R: Read + Seek> WadArchive<R> {
    /// Reads the index and checks every block lies inside the file.
    pub fn open(mut reader: R) -> Result<Self, WadError> {
        let index = WadIndex::read_from(&mut reader)?;
        let len = reader.seek(SeekFrom::End(0))?;
        index.check_file_len(len)?;
        Ok(Self { index, reader })
    }

    pub fn index(&self) -> &WadIndex {
        &self.index
    }

    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Reads a block's stored bytes (its `sges` container plus padding).
    pub fn read_block_raw(&mut self, block: usize) -> Result<Vec<u8>, WadError> {
        let b = self
            .index
            .blocks
            .get(block)
            .ok_or(WadError::NoSuchBlock(block))?;
        // At most 0xFFFF sectors (2 GiB), checked against the file in `open`.
        let mut buf = vec![0; usize::try_from(b.stored_len()).map_err(io::Error::other)?];
        self.reader.seek(SeekFrom::Start(b.offset()))?;
        self.reader.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Reads and decodes a block.
    pub fn read_block(&mut self, block: usize) -> Result<Vec<u8>, WadError> {
        let raw = self.read_block_raw(block)?;
        sges::decode(&raw).map_err(|source| WadError::Block { block, source })
    }

    /// Reads and decodes the block at `path` (see [`WadIndex::find_block`]).
    /// Returns `Ok(None)` if there is no such block.
    pub fn read_block_by_path(&mut self, path: &str) -> Result<Option<Vec<u8>>, WadError> {
        match self.index.find_block(path) {
            Some(block) => self.read_block(block).map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::testutil::{SegmentKind, WadBuilder};

    fn payload(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8 ^ seed).collect()
    }

    fn sample() -> (Vec<u8>, Vec<Vec<u8>>) {
        let blocks = vec![payload(0x2_3456, 1), payload(0x10, 2), payload(0x9000, 3)];
        let mut w = WadBuilder::new(&[0xAAAA_0000, 0xBBBB_0001, 0xCCCC_0002]);
        w.block(
            "blocks\\Test\\first_P000_Q3.block",
            &blocks[0],
            SegmentKind::Deflate,
        );
        w.block(
            "blocks\\Test\\second_P000_Q3.block",
            &blocks[1],
            SegmentKind::Stored,
        );
        w.block(
            "blocks\\Test\\third_P000_Q3.block",
            &blocks[2],
            SegmentKind::Deflate,
        );
        w.asset(0x1111, 0, 2, [NO_BLOCK; 3]);
        w.asset(0x2222, 0, 0, [NO_BLOCK; 3]);
        w.asset(0x3333, 1, 1, [2, NO_BLOCK, NO_BLOCK]);
        w.asset(0x1111, 2, 1, [NO_BLOCK; 3]);
        (w.build(), blocks)
    }

    #[test]
    fn parses_header_and_tables() {
        let (file, _) = sample();
        let index = WadIndex::read_from(&mut Cursor::new(&file)).unwrap();
        let h = index.header();
        assert_eq!(h.block_count, 3);
        assert_eq!(h.asset_count, 4);
        assert_eq!(h.data_offset as u64 % SECTOR_SIZE, 0);
        assert_eq!(index.type_hashes(), [0xAAAA_0000, 0xBBBB_0001, 0xCCCC_0002]);
        assert_eq!(index.blocks()[1].path, "blocks\\Test\\second_P000_Q3.block");
        assert_eq!(index.blocks()[0].sectors, 5);

        // Blocks are contiguous and the last one ends the file.
        let b = index.blocks();
        for w in b.windows(2) {
            assert_eq!(
                w[0].start_sector + u32::from(w[0].stored_sectors),
                w[1].start_sector
            );
        }
        let last = b.last().unwrap();
        assert_eq!(last.offset() + last.stored_len(), file.len() as u64);
    }

    #[test]
    fn looks_up_blocks_and_assets() {
        let (file, _) = sample();
        let index = WadIndex::parse(&file).unwrap();
        assert_eq!(index.find_block("BLOCKS/test/Third_P000_Q3.block"), Some(2));
        assert_eq!(index.find_block("blocks\\Test\\missing.block"), None);

        let in_one: Vec<_> = index.assets_in_block(1).map(|a| a.name_hash).collect();
        assert_eq!(in_one, [0x3333, 0x1111]);
        let named: Vec<_> = index
            .assets_by_hash(0x1111)
            .map(|a| index.type_hash(a))
            .collect();
        assert_eq!(named, [0xAAAA_0000, 0xCCCC_0002]);
        let alt: Vec<_> = index.assets()[2].alt_blocks().collect();
        assert_eq!(alt, [2]);
    }

    #[test]
    fn reads_blocks() {
        let (file, blocks) = sample();
        let mut wad = WadArchive::open(Cursor::new(file)).unwrap();
        for (i, expected) in blocks.iter().enumerate() {
            assert_eq!(&wad.read_block(i).unwrap(), expected, "block {i}");
        }
        assert_eq!(
            wad.read_block_by_path("blocks/test/second_p000_q3.block")
                .unwrap()
                .as_deref(),
            Some(&blocks[1][..])
        );
        assert!(wad.read_block_by_path("nope").unwrap().is_none());
        assert!(matches!(wad.read_block(3), Err(WadError::NoSuchBlock(3))));
    }

    fn put_u32(file: &mut [u8], at: usize, v: u32) {
        file[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn rejects_bad_header() {
        let (file, _) = sample();
        let mut f = file.clone();
        f[0] = b'X';
        assert!(matches!(WadIndex::parse(&f), Err(WadError::BadMagic(_))));
        let mut f = file.clone();
        put_u32(&mut f, 4, 3);
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::UnsupportedVersion(3))
        ));
        let mut f = file.clone();
        f[0x18..0x1C].copy_from_slice(b"DATB");
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::UnexpectedChunk { index: 1, .. })
        ));
        let mut f = file;
        put_u32(&mut f, 0x44, 2);
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::PathCountMismatch { .. })
        ));
    }

    #[test]
    fn rejects_tables_past_data() {
        let (file, _) = sample();
        // INDX block count.
        let mut f = file.clone();
        put_u32(&mut f, 0x14, 0x10_0000);
        put_u32(&mut f, 0x44, 0x10_0000);
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::TableOutOfBounds { table: "INDX", .. })
        ));
        // ASET offset.
        let mut f = file.clone();
        put_u32(&mut f, 0x34, u32::MAX);
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::TableOutOfBounds { table: "ASET", .. })
        ));
        // Huge data offset.
        let mut f = file;
        put_u32(&mut f, 0x1C, u32::MAX);
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::TablesTooLarge(_))
        ));
    }

    #[test]
    fn rejects_truncated_tables() {
        let (file, _) = sample();
        let data = WadIndex::parse(&file).unwrap().header().data_offset as usize;
        for len in [0, 3, HEADER_LEN - 1, HEADER_LEN, data - 1] {
            assert!(
                WadIndex::parse(&file[..len]).is_err(),
                "accepted {len} bytes"
            );
            assert!(
                WadIndex::read_from(&mut Cursor::new(&file[..len])).is_err(),
                "read {len} bytes"
            );
        }
    }

    #[test]
    fn rejects_unterminated_paths() {
        let (mut file, _) = sample();
        let h = WadIndex::parse(&file).unwrap().header().to_owned();
        let data = h.data_offset as usize;
        file[h.paths_offset as usize..data].fill(b'a');
        assert!(matches!(
            WadIndex::parse(&file),
            Err(WadError::Truncated { .. })
        ));
    }

    #[test]
    fn rejects_bad_asset_references() {
        let (file, _) = sample();
        let at = WadIndex::parse(&file).unwrap().header().asset_offset as usize;
        let mut f = file.clone();
        f[at + 10..at + 12].copy_from_slice(&3u16.to_le_bytes());
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::BadBlockIndex {
                asset: 0,
                block: 3,
                ..
            })
        ));
        let mut f = file.clone();
        f[at + 4..at + 6].copy_from_slice(&7u16.to_le_bytes());
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::BadBlockIndex {
                asset: 0,
                block: 7,
                ..
            })
        ));
        let mut f = file;
        put_u32(&mut f, at + 12, 3);
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::BadTypeIndex {
                asset: 0,
                index: 3,
                ..
            })
        ));
    }

    #[test]
    fn rejects_blocks_outside_file() {
        let (file, _) = sample();
        let short = file[..file.len() - SECTOR_SIZE as usize].to_vec();
        assert!(matches!(
            WadArchive::open(Cursor::new(short)),
            Err(WadError::BlockOutOfBounds { block: 2, .. })
        ));
        // A block may not start inside the tables.
        let at = WadIndex::parse(&file).unwrap().header().index_offset as usize;
        let mut f = file;
        put_u32(&mut f, at, 0);
        assert!(matches!(
            WadIndex::parse(&f),
            Err(WadError::BlockOutOfBounds { block: 0, .. })
        ));
    }

    #[test]
    fn corrupt_block_reports_its_index() {
        let (file, _) = sample();
        let start = WadIndex::parse(&file).unwrap().blocks()[1].offset() as usize;
        let mut f = file;
        f[start] = b'x';
        let mut wad = WadArchive::open(Cursor::new(f)).unwrap();
        assert!(matches!(
            wad.read_block(1),
            Err(WadError::Block {
                block: 1,
                source: SgesError::BadMagic(_)
            })
        ));
    }

    /// Single-byte corruption anywhere in the tables or block headers must
    /// produce an error or a result, never a panic.
    #[test]
    fn byte_flips_never_panic() {
        let (file, _) = sample();
        let index = WadIndex::parse(&file).unwrap();
        let tables = 0..index.header().data_offset as usize;
        let block_heads = index.blocks().iter().enumerate().flat_map(|(i, b)| {
            let at = b.offset() as usize;
            (at..at + 0x60).map(move |spot| (spot, Some(i)))
        });
        let spots = tables.map(|spot| (spot, None)).chain(block_heads);
        let mut seed = 0x2545_f491_u32;
        for (at, block) in spots {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let mut f = file.clone();
            f[at] ^= (seed as u8).max(1);
            // Opening covers the tables; decode the block whose header changed.
            if let (Ok(mut wad), Some(i)) = (WadArchive::open(Cursor::new(f)), block) {
                let _ = wad.read_block(i);
            }
        }
    }

    #[test]
    fn names_assets_through_blocks_and_ucfx() {
        use crate::testutil::{UcfxNode, block_contents, ucfx};
        use crate::{BlockContents, Ucfx};

        let named = ucfx(&[
            UcfxNode::Leaf(*b"NAME", b"global_loading_skull\0".to_vec()),
            UcfxNode::Leaf(*b"BODY", vec![7; 0x1000]),
        ]);
        let unnamed = ucfx(&[UcfxNode::Leaf(*b"data", vec![1, 2, 3])]);
        let decoded = block_contents(&[
            (0x443f_889b, 0xf011_157a, 0, named),
            (0x96fb_0f27, 0xBBBB_0001, 0, unnamed),
        ]);
        let mut w = WadBuilder::new(&[0xf011_157a, 0xBBBB_0001]);
        w.block(
            "blocks\\Test\\skull_P000_Q3.block",
            &decoded,
            SegmentKind::Deflate,
        );
        w.asset(0x443f_889b, 0, 0, [NO_BLOCK; 3]);
        w.asset(0x96fb_0f27, 1, 0, [NO_BLOCK; 3]);

        let mut wad = WadArchive::open(Cursor::new(w.build())).unwrap();
        let data = wad.read_block(0).unwrap();
        let block = BlockContents::parse(&data).unwrap();
        let names: Vec<_> = wad
            .index()
            .assets()
            .iter()
            .map(|a| {
                let ty = wad.index().type_hash(a);
                let entry = block.find(a.name_hash, ty).unwrap();
                Ucfx::parse(block.asset_data(entry)).unwrap().name()
            })
            .collect();
        assert_eq!(names, [Some("global_loading_skull"), None]);
    }
}
