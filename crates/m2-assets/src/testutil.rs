//! Builders for synthetic archives used in tests.

use crate::sges::MAX_SEGMENT_LEN;
use crate::wad::SECTOR_SIZE;

#[derive(Debug, Clone, Copy)]
pub enum SegmentKind {
    Stored,
    Deflate,
}

fn align(buf: &mut Vec<u8>, to: usize) {
    buf.resize(buf.len().next_multiple_of(to), 0);
}

/// Builds an `sges` container from `(data, kind)` segments.
pub fn sges(segments: &[(&[u8], SegmentKind)]) -> Vec<u8> {
    let stored: Vec<(Vec<u8>, bool)> = segments
        .iter()
        .map(|&(data, kind)| {
            assert!(data.len() as u32 <= MAX_SEGMENT_LEN);
            match kind {
                SegmentKind::Stored => (data.to_vec(), false),
                SegmentKind::Deflate => (miniz_oxide::deflate::compress_to_vec(data, 6), true),
            }
        })
        .collect();

    let mut body = vec![0; 0x10 + 8 * segments.len()];
    // Fill the gap before the first segment with junk, as the real files do.
    body.resize(body.len().next_multiple_of(16), 0xEE);
    let mut records = Vec::new();
    for ((data, _), (bytes, compressed)) in segments.iter().zip(&stored) {
        let offset = body.len() as u32;
        records.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        records.extend_from_slice(&(data.len() as u16).to_le_bytes());
        records.extend_from_slice(&(offset | u32::from(*compressed)).to_le_bytes());
        body.extend_from_slice(bytes);
        align(&mut body, 16);
    }
    let total: usize = segments.iter().map(|(d, _)| d.len()).sum();
    body[0..4].copy_from_slice(b"sges");
    body[4..6].copy_from_slice(&4u16.to_le_bytes());
    body[6..8].copy_from_slice(&(segments.len() as u16).to_le_bytes());
    body[8..12].copy_from_slice(&(total as u32).to_le_bytes());
    let len = body.len() as u32;
    body[12..16].copy_from_slice(&len.to_le_bytes());
    body[0x10..0x10 + records.len()].copy_from_slice(&records);
    body
}

/// Builds an `FFCS` archive.
pub struct WadBuilder {
    types: Vec<u32>,
    blocks: Vec<(String, Vec<u8>, u32)>,
    assets: Vec<[u8; 16]>,
}

impl WadBuilder {
    pub fn new(types: &[u32]) -> Self {
        Self {
            types: types.to_vec(),
            blocks: Vec::new(),
            assets: Vec::new(),
        }
    }

    /// Adds a block holding `data`, split into 64 KiB segments.
    pub fn block(&mut self, path: &str, data: &[u8], kind: SegmentKind) {
        let segs: Vec<_> = data
            .chunks(MAX_SEGMENT_LEN as usize)
            .map(|c| (c, kind))
            .collect();
        let sectors = (data.len() as u64).div_ceil(SECTOR_SIZE) as u32;
        self.blocks.push((path.to_owned(), sges(&segs), sectors));
    }

    pub fn asset(&mut self, name_hash: u32, type_index: u32, block: u16, alt: [u16; 3]) {
        let mut rec = [0; 16];
        rec[0..4].copy_from_slice(&name_hash.to_le_bytes());
        for (i, a) in alt.iter().enumerate() {
            rec[4 + 2 * i..6 + 2 * i].copy_from_slice(&a.to_le_bytes());
        }
        rec[10..12].copy_from_slice(&block.to_le_bytes());
        rec[12..16].copy_from_slice(&type_index.to_le_bytes());
        self.assets.push(rec);
    }

    pub fn build(&self) -> Vec<u8> {
        let n = self.blocks.len() as u32;
        let indx = 0x48 + 4 * self.types.len() as u32;
        let aset = indx + 12 * n;
        let pths = aset + 16 * self.assets.len() as u32;
        let paths_len: usize = self.blocks.iter().map(|(p, ..)| p.len() + 1).sum();
        let data = (u64::from(pths) + paths_len as u64).next_multiple_of(SECTOR_SIZE) as u32;

        let mut f = Vec::new();
        f.extend_from_slice(b"FFCS");
        for v in [2, 7] {
            f.extend_from_slice(&u32::to_le_bytes(v));
        }
        let chunks: [(&[u8; 4], u32, u32); 5] = [
            (b"INDX", indx, n),
            (b"DATA", data, self.types.len() as u32),
            (b"CSUM", 0x1234_5678, 0),
            (b"ASET", aset, self.assets.len() as u32),
            (b"PTHS", pths, n),
        ];
        for (cc, a, b) in chunks {
            f.extend_from_slice(cc);
            f.extend_from_slice(&a.to_le_bytes());
            f.extend_from_slice(&b.to_le_bytes());
        }
        for t in &self.types {
            f.extend_from_slice(&t.to_le_bytes());
        }

        let mut sector = data / SECTOR_SIZE as u32;
        for (_, body, sectors) in &self.blocks {
            let stored = (body.len() as u64).div_ceil(SECTOR_SIZE) as u32;
            f.extend_from_slice(&sector.to_le_bytes());
            f.extend_from_slice(&sectors.to_le_bytes());
            f.extend_from_slice(&(stored | 0x8000_0000).to_le_bytes());
            sector += stored;
        }
        for a in &self.assets {
            f.extend_from_slice(a);
        }
        for (p, ..) in &self.blocks {
            f.extend_from_slice(p.as_bytes());
            f.push(0);
        }
        f.resize(data as usize, 0);
        for (_, body, _) in &self.blocks {
            f.extend_from_slice(body);
            align(&mut f, SECTOR_SIZE as usize);
        }
        f
    }
}

/// Builds a decoded block from `(name_hash, type_hash, unk_0x08, data)`.
pub fn block_contents(assets: &[(u32, u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = (assets.len() as u32).to_le_bytes().to_vec();
    for (name, ty, unk, data) in assets {
        for v in [*name, *ty, *unk, data.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for (.., data) in assets {
        out.extend_from_slice(data);
    }
    out
}

/// A section for [`ucfx`].
pub enum UcfxNode {
    Leaf([u8; 4], Vec<u8>),
    Container([u8; 4], Vec<UcfxNode>),
}

/// Builds a `UCFX` container from a section tree.
pub fn ucfx(nodes: &[UcfxNode]) -> Vec<u8> {
    fn walk(nodes: &[UcfxNode], records: &mut Vec<u8>, data: &mut Vec<u8>, count: &mut u32) {
        for (i, node) in nodes.iter().enumerate() {
            let after = (nodes.len() - 1 - i) as u32;
            *count += 1;
            match node {
                UcfxNode::Leaf(cc, bytes) => {
                    records.extend_from_slice(cc);
                    for v in [data.len() as u32, bytes.len() as u32, after, 0] {
                        records.extend_from_slice(&v.to_le_bytes());
                    }
                    data.extend_from_slice(bytes);
                }
                UcfxNode::Container(cc, children) => {
                    records.extend_from_slice(cc);
                    for v in [u32::MAX, 0, after, children.len() as u32] {
                        records.extend_from_slice(&v.to_le_bytes());
                    }
                    walk(children, records, data, count);
                }
            }
        }
    }
    let (mut records, mut data, mut count) = (Vec::new(), Vec::new(), 0);
    walk(nodes, &mut records, &mut data, &mut count);
    let mut out = b"UCFX".to_vec();
    for v in [0x14 + 20 * count, 0, 0, count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&records);
    out.extend_from_slice(&data);
    out.extend_from_slice(b"CSUM\0\0\0\0");
    out
}
