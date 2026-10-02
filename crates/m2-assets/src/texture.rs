//! Texture assets: the `INFO` header, the mip chain layout and conversion of
//! mips to RGBA8.
//!
//! A texture asset is a flat `UCFX` container with `NAME`, `INFO` and `BODY`
//! sections. `INFO` is 34 bytes:
//!
//! ```text
//! 0x00  u16      width
//! 0x02  u16      height
//! 0x04  u16      depth (always 1 so far)
//! 0x06  u16      mip count
//! 0x08  u16      flags; 0x20 is set on every specular map, the rest unknown
//! 0x0A  u16      kind: 1 = 2D, 2 = cube
//! 0x0C  u16      unknown (1 or 2)
//! 0x0E  u32      D3DFORMAT ('DXT1', 'DXT5' or 21 = A8R8G8B8)
//! 0x12  f32      probably the mip LOD bias
//! 0x16  u32      bytes in the whole chain, every face and mip
//! 0x1A  u16 x 4  mip masks of quality tiers Q0 to Q3 (bit n = mip n);
//!                0xFFFF in Q3 means every mip
//! ```
//!
//! Each mip is `width >> n` by `height >> n`. Chains stop when the smaller
//! side reaches 4 rather than going down to 1x1, but the header's mip count
//! is authoritative. Mips are stored largest first with no padding. A cube
//! map stores each face's whole chain in turn (+X, -X, +Y, -Y, +Z, -Z).
//!
//! # Streamed mips
//!
//! The tier masks split the chain into up to four runs of mips, Q0 holding
//! the largest. `BODY` holds only the Q3 mips. Each other non-empty tier is a
//! separate asset with the same name and type hash, whose `UCFX` container
//! has a lone `BODY` holding that tier's mips; it is stored in the sibling
//! block whose path ends in `_P001_Q2`, `_P002_Q1` or `_P003_Q0` where the
//! header's block ends in `_P000_Q3`. The full chain is the Q0, Q1 and Q2
//! chunks followed by `BODY`; see [`Texture::assemble`].
//!
//! # Provenance
//!
//! Format-from-samples: inferred from every texture in the four archives of
//! a legally installed copy (13,846 headers and 23,384 streamed chunks), with
//! the chain size, tier partition and chunk lengths checked on all of them
//! and the decoded images inspected locally. No bytes from the archives are
//! committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::dxt::{self, DxtError, DxtFormat};
use crate::ucfx::{Ucfx, UcfxError};

/// Length of the `INFO` section.
pub const INFO_LEN: usize = 0x22;
/// Section holding the header.
pub const INFO: [u8; 4] = *b"INFO";
/// Section holding pixel data.
pub const BODY: [u8; 4] = *b"BODY";
/// Most mips a chain can have, one per mask bit.
pub const MAX_MIPS: u16 = 16;

/// `D3DFMT_A8R8G8B8`.
const D3DFMT_A8R8G8B8: u32 = 21;

/// Errors from parsing or decoding a texture.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TextureError {
    #[error("INFO section is {0:#x} bytes, expected {INFO_LEN:#x}")]
    InfoLength(usize),
    #[error("texture has no {} section", String::from_utf8_lossy(.0))]
    MissingSection([u8; 4]),
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("{width}x{height}x{depth} texture is empty")]
    Empty { width: u16, height: u16, depth: u16 },
    #[error("mip count {0} is outside 1..={MAX_MIPS}")]
    MipCount(u16),
    #[error("unsupported pixel format {0:#010x}")]
    UnsupportedFormat(u32),
    #[error("unsupported texture kind {0}")]
    UnsupportedKind(u16),
    #[error("mip {0} is out of range")]
    MipOutOfRange(u32),
    #[error("tier masks {0:04x?} do not split the chain into whole runs of mips")]
    BadTiers([u16; 4]),
    #[error("chain size field says {field:#x} bytes, the layout needs {computed:#x}")]
    ChainLength { field: u32, computed: u64 },
    #[error("tier Q{tier} needs {needed:#x} bytes, got {got:#x}")]
    TierLength {
        tier: usize,
        needed: u64,
        got: usize,
    },
    #[error("tier Q{0} is streamed but no chunk was given for it")]
    MissingTier(usize),
    #[error("tier Q{0} is empty but a chunk was given for it")]
    UnexpectedTier(usize),
    #[error("streamed mips on a texture with {0} faces")]
    StreamedFaces(u32),
    #[error(transparent)]
    Dxt(#[from] DxtError),
    #[error("A8R8G8B8 mip of {width}x{height} needs {needed:#x} bytes, got {got:#x}")]
    PixelLength {
        width: u32,
        height: u32,
        needed: u64,
        got: usize,
    },
}

/// A pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    Dxt1,
    Dxt3,
    Dxt5,
    /// 32-bit, stored as B, G, R, A bytes.
    A8R8G8B8,
}

impl PixelFormat {
    /// Maps a `D3DFORMAT` value.
    pub fn from_d3d(code: u32) -> Option<Self> {
        match &code.to_le_bytes() {
            b"DXT1" => Some(Self::Dxt1),
            b"DXT3" => Some(Self::Dxt3),
            b"DXT5" => Some(Self::Dxt5),
            _ if code == D3DFMT_A8R8G8B8 => Some(Self::A8R8G8B8),
            _ => None,
        }
    }

    /// The `D3DFORMAT` value.
    pub fn to_d3d(self) -> u32 {
        match self {
            Self::Dxt1 => u32::from_le_bytes(*b"DXT1"),
            Self::Dxt3 => u32::from_le_bytes(*b"DXT3"),
            Self::Dxt5 => u32::from_le_bytes(*b"DXT5"),
            Self::A8R8G8B8 => D3DFMT_A8R8G8B8,
        }
    }

    fn dxt(self) -> Option<DxtFormat> {
        match self {
            Self::Dxt1 => Some(DxtFormat::Dxt1),
            Self::Dxt3 => Some(DxtFormat::Dxt3),
            Self::Dxt5 => Some(DxtFormat::Dxt5),
            Self::A8R8G8B8 => None,
        }
    }

    /// Bytes in one `width` x `height` image.
    pub fn image_len(self, width: u32, height: u32) -> u64 {
        let (w, h) = (u64::from(width), u64::from(height));
        match self.dxt() {
            Some(f) => w.div_ceil(4) * h.div_ceil(4) * f.block_len() as u64,
            None => w * h * 4,
        }
    }
}

/// How the faces of a texture are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureKind {
    Texture2d,
    Cube,
}

impl TextureKind {
    pub fn faces(self) -> u32 {
        match self {
            Self::Texture2d => 1,
            Self::Cube => 6,
        }
    }
}

/// A parsed `INFO` section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureInfo {
    pub width: u16,
    pub height: u16,
    pub depth: u16,
    pub mip_count: u16,
    /// Field at 0x08; 0x20 marks specular maps.
    pub flags: u16,
    pub kind: TextureKind,
    /// Field at 0x0C.
    pub unk_0x0c: u16,
    pub format: PixelFormat,
    /// Field at 0x12; probably the mip LOD bias.
    pub lod_bias: f32,
    /// Bytes in the whole chain, as stored in the header.
    pub chain_len: u32,
    /// Mip masks of tiers Q0 to Q3, with Q3's "every mip" value expanded.
    pub tier_masks: [u16; 4],
}

/// Index of the tier stored in the texture asset's own `BODY`.
pub const RESIDENT_TIER: usize = 3;

impl TextureInfo {
    /// Parses and checks an `INFO` section: the mip count fits the size,
    /// the tiers split the chain into contiguous runs, and the chain size
    /// field matches the layout.
    pub fn parse(info: &[u8]) -> Result<Self, TextureError> {
        if info.len() != INFO_LEN {
            return Err(TextureError::InfoLength(info.len()));
        }
        let mut r = Reader::at(info, 0);
        let read = |r: &mut Reader| -> Result<_, OutOfBounds> {
            Ok((
                [
                    r.u16()?,
                    r.u16()?,
                    r.u16()?,
                    r.u16()?,
                    r.u16()?,
                    r.u16()?,
                    r.u16()?,
                ],
                r.u32()?,
                f32::from_bits(r.u32()?),
                r.u32()?,
                [r.u16()?, r.u16()?, r.u16()?, r.u16()?],
            ))
        };
        let Ok((
            [width, height, depth, mip_count, flags, kind, unk_0x0c],
            format,
            lod_bias,
            chain_len,
            mut tier_masks,
        )) = read(&mut r)
        else {
            unreachable!("INFO length was checked");
        };

        if width == 0 || height == 0 || depth == 0 {
            return Err(TextureError::Empty {
                width,
                height,
                depth,
            });
        }
        if !(1..=MAX_MIPS).contains(&mip_count) {
            return Err(TextureError::MipCount(mip_count));
        }
        // Every mip must be at least 1x1.
        if u32::from(mip_count) > 16 - width.max(height).leading_zeros() {
            return Err(TextureError::MipCount(mip_count));
        }
        let kind = match kind {
            1 => TextureKind::Texture2d,
            2 => TextureKind::Cube,
            k => return Err(TextureError::UnsupportedKind(k)),
        };
        let format =
            PixelFormat::from_d3d(format).ok_or(TextureError::UnsupportedFormat(format))?;

        let all = mask_below(mip_count);
        if tier_masks[RESIDENT_TIER] == 0xFFFF {
            tier_masks[RESIDENT_TIER] = all;
        }
        let raw_masks = tier_masks;
        // Tiers must be disjoint, cover the chain, and each be one run of
        // mips, with lower tiers holding larger mips.
        let mut next = 0u32;
        for &m in &tier_masks {
            if m == 0 {
                continue;
            }
            let len = m.count_ones();
            if m.trailing_zeros() != next || u32::from(m >> next) != (1 << len) - 1 {
                return Err(TextureError::BadTiers(raw_masks));
            }
            next += len;
        }
        if next != u32::from(mip_count) {
            return Err(TextureError::BadTiers(raw_masks));
        }

        let info = Self {
            width,
            height,
            depth,
            mip_count,
            flags,
            kind,
            unk_0x0c,
            format,
            lod_bias,
            chain_len,
            tier_masks,
        };
        let computed = info.mask_len(all) * u64::from(info.faces());
        if computed != u64::from(chain_len) {
            return Err(TextureError::ChainLength {
                field: chain_len,
                computed,
            });
        }
        if info.faces() > 1 && tier_masks[RESIDENT_TIER] != all {
            return Err(TextureError::StreamedFaces(info.faces()));
        }
        Ok(info)
    }

    pub fn faces(&self) -> u32 {
        self.kind.faces()
    }

    /// Size of mip `level`.
    pub fn mip_size(&self, level: u32) -> Result<(u32, u32), TextureError> {
        if level >= u32::from(self.mip_count) {
            return Err(TextureError::MipOutOfRange(level));
        }
        Ok((
            (u32::from(self.width) >> level).max(1),
            (u32::from(self.height) >> level).max(1),
        ))
    }

    /// Bytes in one face of mip `level`.
    pub fn mip_len(&self, level: u32) -> Result<u64, TextureError> {
        let (w, h) = self.mip_size(level)?;
        Ok(self.format.image_len(w, h))
    }

    /// Whether tier `tier` (0 to 3) holds any mips.
    pub fn has_tier(&self, tier: usize) -> bool {
        self.tier_masks.get(tier).is_some_and(|&m| m != 0)
    }

    /// Whether every mip is in the texture asset's own `BODY`.
    pub fn is_resident(&self) -> bool {
        self.tier_masks[RESIDENT_TIER] == mask_below(self.mip_count)
    }

    /// Bytes in tier `tier` (0 to 3), every face.
    pub fn tier_len(&self, tier: usize) -> u64 {
        self.tier_masks
            .get(tier)
            .map_or(0, |&m| self.mask_len(m) * u64::from(self.faces()))
    }

    /// Bytes in one face of the mips in `mask`.
    fn mask_len(&self, mask: u16) -> u64 {
        (0..u32::from(self.mip_count))
            .filter(|&m| mask & (1 << m) != 0)
            .map(|m| {
                let (w, h) = (
                    (u32::from(self.width) >> m).max(1),
                    (u32::from(self.height) >> m).max(1),
                );
                self.format.image_len(w, h)
            })
            .sum()
    }
}

fn mask_below(count: u16) -> u16 {
    // count is at most 16.
    (((1u32) << count) - 1) as u16
}

/// One face of one mip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mip<'a> {
    pub level: u32,
    /// 0 for 2D textures; 0 to 5 for cube maps.
    pub face: u32,
    pub width: u32,
    pub height: u32,
    pub data: &'a [u8],
}

/// A texture asset: its name, header and resident pixel data.
#[derive(Debug, Clone)]
pub struct Texture<'a> {
    pub name: Option<&'a str>,
    pub info: TextureInfo,
    /// The Q3 mips.
    pub body: &'a [u8],
}

impl<'a> Texture<'a> {
    /// Parses a texture asset and checks `BODY` holds exactly the Q3 mips.
    pub fn parse(asset: &'a [u8]) -> Result<Self, TextureError> {
        let ucfx = Ucfx::parse(asset)?;
        let info = ucfx
            .top_level(INFO)
            .ok_or(TextureError::MissingSection(INFO))?;
        let info = TextureInfo::parse(info)?;
        let body = ucfx
            .top_level(BODY)
            .ok_or(TextureError::MissingSection(BODY))?;
        check_tier(&info, RESIDENT_TIER, body)?;
        Ok(Self {
            name: ucfx.name(),
            info,
            body,
        })
    }

    /// Mips held in `BODY`, largest first (face by face for cube maps).
    pub fn resident_mips(&self) -> Vec<Mip<'a>> {
        split(&self.info, self.info.tier_masks[RESIDENT_TIER], self.body)
    }

    /// Joins the streamed chunks for tiers Q0 to Q2 (each the `BODY` of the
    /// chunk asset, `None` for empty tiers) with `BODY` into the whole chain.
    pub fn assemble(&self, chunks: [Option<&[u8]>; 3]) -> Result<Vec<u8>, TextureError> {
        let mut chain = Vec::with_capacity(self.info.chain_len as usize);
        for (tier, chunk) in chunks.into_iter().enumerate() {
            match (self.info.has_tier(tier), chunk) {
                (true, Some(c)) => {
                    check_tier(&self.info, tier, c)?;
                    chain.extend_from_slice(c);
                }
                (true, None) => return Err(TextureError::MissingTier(tier)),
                (false, Some(_)) => return Err(TextureError::UnexpectedTier(tier)),
                (false, None) => {}
            }
        }
        chain.extend_from_slice(self.body);
        Ok(chain)
    }

    /// Splits a whole chain from [`Texture::assemble`] into mips.
    pub fn chain_mips<'c>(&self, chain: &'c [u8]) -> Result<Vec<Mip<'c>>, TextureError> {
        let all = mask_below(self.info.mip_count);
        if chain.len() as u64 != u64::from(self.info.chain_len) {
            return Err(TextureError::ChainLength {
                field: self.info.chain_len,
                computed: chain.len() as u64,
            });
        }
        Ok(split(&self.info, all, chain))
    }
}

/// Checks `data` is exactly the mips of tier `tier`, every face.
fn check_tier(info: &TextureInfo, tier: usize, data: &[u8]) -> Result<(), TextureError> {
    let needed = info.tier_len(tier);
    if data.len() as u64 != needed {
        return Err(TextureError::TierLength {
            tier,
            needed,
            got: data.len(),
        });
    }
    Ok(())
}

/// Splits `data`, already checked to be exactly the mips in `mask` for every
/// face, into mips.
fn split<'d>(info: &TextureInfo, mask: u16, data: &'d [u8]) -> Vec<Mip<'d>> {
    let mut mips = Vec::new();
    let mut at = 0usize;
    for face in 0..info.faces() {
        for level in (0..u32::from(info.mip_count)).filter(|&m| mask & (1 << m) != 0) {
            let (width, height) = (
                (u32::from(info.width) >> level).max(1),
                (u32::from(info.height) >> level).max(1),
            );
            // Fits: the total was checked against data.len().
            let len = info.format.image_len(width, height) as usize;
            mips.push(Mip {
                level,
                face,
                width,
                height,
                data: &data[at..at + len],
            });
            at += len;
        }
    }
    mips
}

/// Decodes one mip to tightly packed RGBA8, rows top first.
pub fn to_rgba8(
    format: PixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u8>, TextureError> {
    match format.dxt() {
        Some(f) => Ok(dxt::decode(f, width, height, data)?),
        None => {
            let needed = format.image_len(width, height);
            if data.len() as u64 != needed {
                return Err(TextureError::PixelLength {
                    width,
                    height,
                    needed,
                    got: data.len(),
                });
            }
            Ok(data
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], p[3]])
                .collect())
        }
    }
}

impl Mip<'_> {
    /// Decodes this mip to RGBA8.
    pub fn to_rgba8(&self, format: PixelFormat) -> Result<Vec<u8>, TextureError> {
        to_rgba8(format, self.width, self.height, self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    struct Header {
        width: u16,
        height: u16,
        mips: u16,
        kind: u16,
        format: u32,
        chain_len: u32,
        tiers: [u16; 4],
    }

    impl Header {
        fn dxt1(width: u16, height: u16, mips: u16) -> Self {
            let mut h = Self {
                width,
                height,
                mips,
                kind: 1,
                format: u32::from_le_bytes(*b"DXT1"),
                chain_len: 0,
                tiers: [0, 0, 0, 0xFFFF],
            };
            h.chain_len = (0..mips)
                .map(|m| {
                    PixelFormat::Dxt1.image_len(
                        (u32::from(width) >> m).max(1),
                        (u32::from(height) >> m).max(1),
                    ) as u32
                })
                .sum();
            h
        }

        fn bytes(&self) -> Vec<u8> {
            let mut b = Vec::new();
            for v in [self.width, self.height, 1, self.mips, 0x20, self.kind, 1] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&self.format.to_le_bytes());
            b.extend_from_slice(&(-1.0f32).to_le_bytes());
            b.extend_from_slice(&self.chain_len.to_le_bytes());
            for v in self.tiers {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b
        }
    }

    fn asset(h: &Header, body: &[u8]) -> Vec<u8> {
        ucfx(&[
            UcfxNode::Leaf(*b"NAME", b"synthetic_dm\0".to_vec()),
            UcfxNode::Leaf(*b"INFO", h.bytes()),
            UcfxNode::Leaf(*b"BODY", body.to_vec()),
        ])
    }

    /// A DXT1 block of one solid RGB565 colour.
    fn solid(c: u16) -> [u8; 8] {
        let [lo, hi] = c.to_le_bytes();
        [lo, hi, lo, hi, 0, 0, 0, 0]
    }

    #[test]
    fn parses_header_fields() {
        let h = Header::dxt1(64, 32, 4);
        let info = TextureInfo::parse(&h.bytes()).unwrap();
        assert_eq!(
            (info.width, info.height, info.depth, info.mip_count),
            (64, 32, 1, 4)
        );
        assert_eq!(info.format, PixelFormat::Dxt1);
        assert_eq!(info.kind, TextureKind::Texture2d);
        assert_eq!(info.flags, 0x20);
        assert_eq!(info.lod_bias, -1.0);
        // 64x32, 32x16, 16x8, 8x4 in 4x4 blocks of 8 bytes.
        assert_eq!(info.chain_len, (128 + 32 + 8 + 2) * 8);
        assert_eq!(info.tier_masks, [0, 0, 0, 0xF]);
        assert!(info.is_resident());
        assert_eq!(info.mip_size(3), Ok((8, 4)));
        assert_eq!(info.mip_len(3), Ok(16));
        assert!(info.mip_size(4).is_err());
    }

    #[test]
    fn format_codes_round_trip() {
        for f in [
            PixelFormat::Dxt1,
            PixelFormat::Dxt3,
            PixelFormat::Dxt5,
            PixelFormat::A8R8G8B8,
        ] {
            assert_eq!(PixelFormat::from_d3d(f.to_d3d()), Some(f));
        }
        assert_eq!(PixelFormat::from_d3d(50), None);
        assert_eq!(PixelFormat::A8R8G8B8.image_len(3, 2), 24);
        assert_eq!(PixelFormat::Dxt5.image_len(8, 2), 32);
    }

    #[test]
    fn splits_resident_mips_largest_first() {
        let h = Header::dxt1(8, 8, 2);
        let mut body = Vec::new();
        body.extend((0..4).flat_map(|_| solid(0xF800)));
        body.extend(solid(0x001F));
        let a = asset(&h, &body);
        let t = Texture::parse(&a).unwrap();
        assert_eq!(t.name, Some("synthetic_dm"));
        let mips = t.resident_mips();
        let shape: Vec<_> = mips
            .iter()
            .map(|m| (m.level, m.width, m.height, m.data.len()))
            .collect();
        assert_eq!(shape, [(0, 8, 8, 32), (1, 4, 4, 8)]);
        let rgba = mips[1].to_rgba8(t.info.format).unwrap();
        assert_eq!(rgba.len(), 64);
        assert!(rgba.chunks(4).all(|p| p == [0, 0, 255, 255]));
        let rgba = mips[0].to_rgba8(t.info.format).unwrap();
        assert!(rgba.chunks(4).all(|p| p == [255, 0, 0, 255]));
    }

    #[test]
    fn assembles_streamed_tiers() {
        // 16x16 with three mips: mip 0 in Q1, mip 1 in Q2, mip 2 in BODY.
        let mut h = Header::dxt1(16, 16, 3);
        h.tiers = [0, 0b001, 0b010, 0b100];
        let q1 = vec![1; 16 * 8];
        let q2 = vec![2; 4 * 8];
        let body = vec![3; 8];
        let a = asset(&h, &body);
        let t = Texture::parse(&a).unwrap();
        assert!(!t.info.is_resident());
        assert_eq!(
            t.resident_mips()
                .iter()
                .map(|m| m.level)
                .collect::<Vec<_>>(),
            [2]
        );

        assert_eq!(
            t.assemble([None, None, Some(&q2)]),
            Err(TextureError::MissingTier(1))
        );
        assert_eq!(
            t.assemble([Some(&q1), Some(&q1), Some(&q2)]),
            Err(TextureError::UnexpectedTier(0))
        );
        assert!(matches!(
            t.assemble([None, Some(&q2), Some(&q2)]),
            Err(TextureError::TierLength { tier: 1, .. })
        ));
        let chain = t.assemble([None, Some(&q1), Some(&q2)]).unwrap();
        let mips = t.chain_mips(&chain).unwrap();
        let firsts: Vec<_> = mips
            .iter()
            .map(|m| (m.level, m.data[0], m.data.len()))
            .collect();
        assert_eq!(firsts, [(0, 1, 128), (1, 2, 32), (2, 3, 8)]);
        assert!(t.chain_mips(&chain[1..]).is_err());
    }

    #[test]
    fn cube_maps_store_faces_in_turn() {
        let mut h = Header::dxt1(8, 8, 2);
        h.kind = 2;
        h.chain_len *= 6;
        let body: Vec<u8> = (0..6u8).flat_map(|f| vec![f; 40]).collect();
        let a = asset(&h, &body);
        let t = Texture::parse(&a).unwrap();
        let mips = t.resident_mips();
        assert_eq!(mips.len(), 12);
        assert_eq!((mips[2].face, mips[2].level, mips[2].data[0]), (1, 0, 1));
        assert_eq!((mips[11].face, mips[11].level, mips[11].data[0]), (5, 1, 5));

        // A cube map with streamed mips is not a layout we have seen.
        h.tiers = [0, 0, 1, 2];
        assert_eq!(
            TextureInfo::parse(&h.bytes()),
            Err(TextureError::StreamedFaces(6))
        );
    }

    #[test]
    fn converts_a8r8g8b8() {
        let data = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(
            to_rgba8(PixelFormat::A8R8G8B8, 2, 1, &data).unwrap(),
            [3, 2, 1, 4, 7, 6, 5, 8]
        );
        assert!(matches!(
            to_rgba8(PixelFormat::A8R8G8B8, 2, 2, &data),
            Err(TextureError::PixelLength { .. })
        ));
    }

    #[test]
    fn rejects_bad_headers() {
        let good = Header::dxt1(16, 16, 3);
        assert!(TextureInfo::parse(&good.bytes()).is_ok());
        assert_eq!(
            TextureInfo::parse(&good.bytes()[1..]),
            Err(TextureError::InfoLength(0x21))
        );

        let mut h = Header::dxt1(16, 16, 3);
        h.width = 0;
        assert!(matches!(
            TextureInfo::parse(&h.bytes()),
            Err(TextureError::Empty { .. })
        ));
        let mut h = Header::dxt1(16, 16, 3);
        h.mips = 6;
        assert_eq!(
            TextureInfo::parse(&h.bytes()),
            Err(TextureError::MipCount(6))
        );
        h.mips = 0;
        assert_eq!(
            TextureInfo::parse(&h.bytes()),
            Err(TextureError::MipCount(0))
        );
        let mut h = Header::dxt1(16, 16, 3);
        h.format = 50;
        assert_eq!(
            TextureInfo::parse(&h.bytes()),
            Err(TextureError::UnsupportedFormat(50))
        );
        let mut h = Header::dxt1(16, 16, 3);
        h.kind = 3;
        assert_eq!(
            TextureInfo::parse(&h.bytes()),
            Err(TextureError::UnsupportedKind(3))
        );
        let mut h = Header::dxt1(16, 16, 3);
        h.chain_len += 1;
        assert!(matches!(
            TextureInfo::parse(&h.bytes()),
            Err(TextureError::ChainLength { .. })
        ));
    }

    #[test]
    fn rejects_bad_tiers() {
        for tiers in [
            [0, 0, 0, 0b011],       // misses mip 2
            [0, 0, 0b011, 0b110],   // overlap
            [0, 0, 0b101, 0b010],   // not contiguous
            [0, 0, 0b100, 0b011],   // smaller mips in the lower tier
            [0, 0, 0b0011, 0b1100], // past the chain
        ] {
            let mut h = Header::dxt1(16, 16, 3);
            h.tiers = tiers;
            assert_eq!(
                TextureInfo::parse(&h.bytes()),
                Err(TextureError::BadTiers(tiers)),
                "{tiers:?}"
            );
        }
    }

    #[test]
    fn rejects_body_of_wrong_length() {
        let h = Header::dxt1(8, 8, 2);
        let a = asset(&h, &[0; 39]);
        assert!(matches!(
            Texture::parse(&a),
            Err(TextureError::TierLength {
                tier: 3,
                needed: 40,
                got: 39
            })
        ));
        let a = ucfx(&[UcfxNode::Leaf(*b"INFO", h.bytes())]);
        assert_eq!(
            Texture::parse(&a).unwrap_err(),
            TextureError::MissingSection(BODY)
        );
    }

    #[test]
    fn byte_flips_never_panic() {
        let mut h = Header::dxt1(16, 8, 3);
        h.tiers = [0, 0b001, 0b010, 0b100];
        let body = vec![0xAB; 8];
        let good = asset(&h, &body);
        assert!(Texture::parse(&good).is_ok());
        for at in 0..good.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut a = good.clone();
                a[at] = v;
                if let Ok(t) = Texture::parse(&a) {
                    for m in t.resident_mips() {
                        let _ = m.to_rgba8(t.info.format);
                    }
                    let _ = t.assemble([None, Some(&[0; 64]), Some(&[0; 16])]);
                }
            }
        }
    }
}
