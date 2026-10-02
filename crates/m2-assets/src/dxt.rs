//! Decoding of the block-compressed DXT1, DXT3 and DXT5 formats to RGBA8.
//!
//! Implemented from the public format description (Direct3D 9 `D3DFMT_DXT*`,
//! also known as BC1 to BC3). Every 4x4 pixel block is stored as 8 bytes
//! (DXT1) or 16 bytes (DXT3, DXT5), row-major over the image, and images
//! whose sides are not multiples of 4 are padded out to whole blocks.

/// A block-compressed format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DxtFormat {
    /// Two RGB565 endpoints and 2-bit indices; 1-bit alpha when the first
    /// endpoint is not greater than the second.
    Dxt1,
    /// DXT1 colour block plus explicit 4-bit alpha.
    Dxt3,
    /// DXT1 colour block plus interpolated 8-bit alpha.
    Dxt5,
}

impl DxtFormat {
    /// Bytes per 4x4 block.
    pub const fn block_len(self) -> usize {
        match self {
            Self::Dxt1 => 8,
            Self::Dxt3 | Self::Dxt5 => 16,
        }
    }

    /// Bytes needed for a `width` x `height` image, or `None` on overflow.
    pub fn image_len(self, width: u32, height: u32) -> Option<usize> {
        let blocks_x = usize::try_from(width.div_ceil(4)).ok()?;
        let blocks_y = usize::try_from(height.div_ceil(4)).ok()?;
        blocks_x
            .checked_mul(blocks_y)?
            .checked_mul(self.block_len())
    }
}

/// Errors from decoding a block-compressed image.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DxtError {
    #[error("{format:?} image of {width}x{height} needs {needed:#x} bytes, got {got:#x}")]
    WrongLength {
        format: DxtFormat,
        width: u32,
        height: u32,
        needed: usize,
        got: usize,
    },
    #[error("{width}x{height} image is too large to decode")]
    TooLarge { width: u32, height: u32 },
}

/// Decodes a whole image to tightly packed RGBA8, `width * height * 4` bytes.
///
/// `data` must be exactly the image's size in blocks.
pub fn decode(
    format: DxtFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u8>, DxtError> {
    let too_large = DxtError::TooLarge { width, height };
    let needed = format.image_len(width, height).ok_or(too_large.clone())?;
    if data.len() != needed {
        return Err(DxtError::WrongLength {
            format,
            width,
            height,
            needed,
            got: data.len(),
        });
    }
    let (w, h) = (width as usize, height as usize);
    let out_len = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(4))
        .ok_or(too_large)?;
    let mut out = vec![0; out_len];
    let blocks_x = w.div_ceil(4);
    for (i, block) in data.chunks_exact(format.block_len()).enumerate() {
        let pixels = decode_block(format, block);
        let (bx, by) = (i % blocks_x * 4, i / blocks_x * 4);
        for (j, px) in pixels.iter().enumerate() {
            let (x, y) = (bx + j % 4, by + j / 4);
            if x < w && y < h {
                let at = (y * w + x) * 4;
                out[at..at + 4].copy_from_slice(px);
            }
        }
    }
    Ok(out)
}

/// Decodes one block to its 16 pixels in row-major order.
fn decode_block(format: DxtFormat, block: &[u8]) -> [[u8; 4]; 16] {
    match format {
        DxtFormat::Dxt1 => colour_block(block, true),
        DxtFormat::Dxt3 => {
            let mut px = colour_block(&block[8..], false);
            for (i, p) in px.iter_mut().enumerate() {
                let nibble = (block[i / 2] >> (4 * (i % 2))) & 0xF;
                p[3] = nibble * 0x11;
            }
            px
        }
        DxtFormat::Dxt5 => {
            let mut px = colour_block(&block[8..], false);
            let alphas = alpha_palette(block[0], block[1]);
            let mut bits = 0u64;
            for (k, &b) in block[2..8].iter().enumerate() {
                bits |= u64::from(b) << (8 * k);
            }
            for (i, p) in px.iter_mut().enumerate() {
                p[3] = alphas[((bits >> (3 * i)) & 7) as usize];
            }
            px
        }
    }
}

/// Decodes an 8-byte colour block. With `punch_through`, a block whose first
/// endpoint is not greater than the second uses three colours and
/// transparent black (DXT1 only; DXT3 and DXT5 always use four colours).
fn colour_block(block: &[u8], punch_through: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u32, wb: u32, d: u32| -> [u8; 4] {
        let ch = |i: usize| ((u32::from(a[i]) * wa + u32::from(b[i]) * wb) / d) as u8;
        [ch(0), ch(1), ch(2), 0xFF]
    };
    let palette = if c0 > c1 || !punch_through {
        [a, b, mix(2, 1, 3), mix(1, 2, 3)]
    } else {
        [a, b, mix(1, 1, 2), [0, 0, 0, 0]]
    };
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|i| palette[((indices >> (2 * i)) & 3) as usize])
}

/// Expands RGB565 to RGBA8 by replicating the high bits into the low ones.
fn rgb565(c: u16) -> [u8; 4] {
    let r = ((c >> 11) & 0x1F) as u8;
    let g = ((c >> 5) & 0x3F) as u8;
    let b = (c & 0x1F) as u8;
    [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2, 0xFF]
}

/// The eight DXT5 alpha values for two endpoints.
fn alpha_palette(a0: u8, a1: u8) -> [u8; 8] {
    let (x, y) = (u32::from(a0), u32::from(a1));
    let lerp = |wa: u32, wb: u32, d: u32| ((x * wa + y * wb) / d) as u8;
    if a0 > a1 {
        [
            a0,
            a1,
            lerp(6, 1, 7),
            lerp(5, 2, 7),
            lerp(4, 3, 7),
            lerp(3, 4, 7),
            lerp(2, 5, 7),
            lerp(1, 6, 7),
        ]
    } else {
        [
            a0,
            a1,
            lerp(4, 1, 5),
            lerp(3, 2, 5),
            lerp(2, 3, 5),
            lerp(1, 4, 5),
            0,
            0xFF,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: u16 = 0xF800;
    const BLUE: u16 = 0x001F;

    fn bc1(c0: u16, c1: u16, indices: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&c0.to_le_bytes());
        b.extend_from_slice(&c1.to_le_bytes());
        b.extend_from_slice(&indices.to_le_bytes());
        b
    }

    #[test]
    fn sizes_round_up_to_whole_blocks() {
        assert_eq!(DxtFormat::Dxt1.image_len(4, 4), Some(8));
        assert_eq!(DxtFormat::Dxt1.image_len(1, 1), Some(8));
        assert_eq!(DxtFormat::Dxt5.image_len(5, 4), Some(32));
        assert_eq!(DxtFormat::Dxt3.image_len(256, 128), Some(256 * 128));
        assert_eq!(DxtFormat::Dxt1.image_len(0, 4), Some(0));
    }

    #[test]
    fn dxt1_four_colour_block() {
        // Pixel i uses index i % 4: red, blue, 2/3 red, 1/3 red.
        let idx = (0..16).fold(0u32, |acc, i| acc | ((i % 4) << (2 * i)));
        let out = decode(DxtFormat::Dxt1, 4, 4, &bc1(RED, BLUE, idx)).unwrap();
        assert_eq!(&out[0..4], &[255, 0, 0, 255]);
        assert_eq!(&out[4..8], &[0, 0, 255, 255]);
        assert_eq!(&out[8..12], &[170, 0, 85, 255]);
        assert_eq!(&out[12..16], &[85, 0, 170, 255]);
    }

    #[test]
    fn dxt1_punch_through_alpha() {
        // c0 <= c1 selects three colours plus transparent black.
        let idx = (0..16).fold(0u32, |acc, i| acc | ((i % 4) << (2 * i)));
        let out = decode(DxtFormat::Dxt1, 4, 4, &bc1(BLUE, RED, idx)).unwrap();
        assert_eq!(&out[8..12], &[127, 0, 127, 255]);
        assert_eq!(&out[12..16], &[0, 0, 0, 0]);
    }

    #[test]
    fn dxt3_explicit_alpha() {
        let mut block: Vec<u8> = (0..8).map(|i| (2 * i) | ((2 * i + 1) << 4)).collect();
        block.extend(bc1(RED, BLUE, 0));
        let out = decode(DxtFormat::Dxt3, 4, 4, &block).unwrap();
        let alpha: Vec<u8> = out.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alpha, (0..16).map(|i| i * 0x11).collect::<Vec<u8>>());
        assert!(out.chunks(4).all(|p| p[..3] == [255, 0, 0]));
    }

    #[test]
    fn dxt5_interpolated_alpha() {
        // Pixel i uses alpha index i % 8.
        let bits = (0..16).fold(0u64, |acc, i| acc | ((i % 8) << (3 * i)));
        let mut block = vec![0xFF, 0x00];
        block.extend_from_slice(&bits.to_le_bytes()[..6]);
        block.extend(bc1(RED, BLUE, 0));
        let out = decode(DxtFormat::Dxt5, 4, 4, &block).unwrap();
        let alpha: Vec<u8> = out.chunks(4).take(8).map(|p| p[3]).collect();
        assert_eq!(alpha, [255, 0, 218, 182, 145, 109, 72, 36]);

        // a0 <= a1 has explicit 0 and 255 entries.
        let mut block = vec![0x00, 0xFF];
        block.extend_from_slice(&bits.to_le_bytes()[..6]);
        block.extend(bc1(RED, BLUE, 0));
        let out = decode(DxtFormat::Dxt5, 4, 4, &block).unwrap();
        let alpha: Vec<u8> = out.chunks(4).take(8).map(|p| p[3]).collect();
        assert_eq!(alpha, [0, 255, 51, 102, 153, 204, 0, 255]);
    }

    #[test]
    fn places_blocks_and_crops_partial_ones() {
        // 6x2 image: two blocks across, cropped to 2 columns and 2 rows of
        // the second.
        let mut data = bc1(RED, RED, 0);
        data.extend(bc1(BLUE, BLUE, 0));
        let out = decode(DxtFormat::Dxt1, 6, 2, &data).unwrap();
        assert_eq!(out.len(), 6 * 2 * 4);
        let row: Vec<_> = out[..24].chunks(4).map(|p| p[0]).collect();
        assert_eq!(row, [255, 255, 255, 255, 0, 0]);
        assert_eq!(&out[44..48], &[0, 0, 255, 255]);
    }

    #[test]
    fn rejects_wrong_lengths() {
        assert_eq!(
            decode(DxtFormat::Dxt1, 8, 4, &[0; 8]),
            Err(DxtError::WrongLength {
                format: DxtFormat::Dxt1,
                width: 8,
                height: 4,
                needed: 16,
                got: 8
            })
        );
        assert!(decode(DxtFormat::Dxt5, 4, 4, &[0; 17]).is_err());
        assert!(decode(DxtFormat::Dxt1, u32::MAX, u32::MAX, &[]).is_err());
    }
}
