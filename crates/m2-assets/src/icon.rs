//! The application icon of a Windows program, read from its file.
//!
//! The game's window and taskbar show the original game's own icon, read
//! at run time from the player's installed executable (never shipped with
//! this project). The executable is only read as a file here: its
//! resource section is found through the PE headers, the first icon group
//! (`RT_GROUP_ICON`, the one Explorer shows for the file) lists its
//! images by size, and each image (`RT_ICON`) is a DIB with a 1-bit
//! transparency mask, or a PNG, as in an `.ico` file (which [`ico_file`]
//! reads too: the install keeps a copy of the icon beside the program).
//!
//! From the PE/COFF and ICO formats (Microsoft's documentation); nothing
//! here is the game's own code.

use miniz_oxide::inflate::decompress_to_vec_zlib;

/// An icon image, 8-bit RGBA, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Why no icon could be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IconError {
    #[error("not a PE file")]
    NotPe,
    #[error("not an .ico file")]
    NotIco,
    #[error("the file has no resource section")]
    NoResources,
    #[error("the file has no icon")]
    NoIcon,
    #[error("the resources are malformed")]
    Malformed,
}

/// Resource types.
const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;

/// The images of the program's own icon (its first icon group), each one
/// that could be decoded, in the group's order.
pub fn program_icon(file: &[u8]) -> Result<Vec<IconImage>, IconError> {
    let pe = Pe::parse(file)?;
    let group = pe
        .first_resource(RT_GROUP_ICON, None)?
        .ok_or(IconError::NoIcon)?;
    let mut images = Vec::new();
    for entry in group_entries(group)? {
        let Some(bytes) = pe.first_resource(RT_ICON, Some(entry.id))? else {
            continue;
        };
        if let Some(image) = decode_image(bytes) {
            images.push(image);
        }
    }
    if images.is_empty() {
        return Err(IconError::NoIcon);
    }
    Ok(images)
}

/// The images of an `.ico` file that could be decoded, in its order.
pub fn ico_file(file: &[u8]) -> Result<Vec<IconImage>, IconError> {
    if file.get(..4) != Some(&[0, 0, 1, 0]) {
        return Err(IconError::NotIco);
    }
    let count = u16_at(file, 4).ok_or(IconError::NotIco)? as usize;
    let mut images = Vec::new();
    for i in 0..count {
        let e = 6 + i * 16;
        let size = u32_at(file, e + 8).ok_or(IconError::Malformed)? as usize;
        let at = u32_at(file, e + 12).ok_or(IconError::Malformed)? as usize;
        let bytes = file.get(at..at + size).ok_or(IconError::Malformed)?;
        if let Some(image) = decode_image(bytes) {
            images.push(image);
        }
    }
    if images.is_empty() {
        return Err(IconError::NoIcon);
    }
    Ok(images)
}

/// Of `images`, the largest no wider than `max` (else the smallest).
pub fn largest_up_to(images: &[IconImage], max: u32) -> Option<&IconImage> {
    images
        .iter()
        .filter(|i| i.width <= max)
        .max_by_key(|i| i.width * i.height)
        .or_else(|| images.iter().min_by_key(|i| i.width * i.height))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A section: where its data is in memory (RVA) and in the file.
struct Section {
    rva: u32,
    size: u32,
    file_offset: u32,
    file_size: u32,
}

/// What of a PE file the resources need.
struct Pe<'a> {
    file: &'a [u8],
    sections: Vec<Section>,
    /// The resource directory's root, as a slice of the file from there on.
    root: usize,
}

impl<'a> Pe<'a> {
    fn parse(file: &'a [u8]) -> Result<Self, IconError> {
        if file.get(..2) != Some(b"MZ") {
            return Err(IconError::NotPe);
        }
        let pe = u32_at(file, 0x3C).ok_or(IconError::NotPe)? as usize;
        if file.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(IconError::NotPe);
        }
        let coff = pe + 4;
        let sections = u16_at(file, coff + 2).ok_or(IconError::NotPe)? as usize;
        let optional_size = u16_at(file, coff + 16).ok_or(IconError::NotPe)? as usize;
        let optional = coff + 20;
        // The data directories' count and place: PE32, PE32+.
        let (count_at, dirs_at) = match u16_at(file, optional).ok_or(IconError::NotPe)? {
            0x10B => (92, 96),
            0x20B => (108, 112),
            _ => return Err(IconError::NotPe),
        };
        let dirs = u32_at(file, optional + count_at).ok_or(IconError::NotPe)?;
        if dirs < 3 {
            return Err(IconError::NoResources);
        }
        // Directory 2: the resources.
        let resource_rva = u32_at(file, optional + dirs_at + 2 * 8).ok_or(IconError::NotPe)?;
        if resource_rva == 0 {
            return Err(IconError::NoResources);
        }
        let table = optional + optional_size;
        let sections = (0..sections)
            .map(|i| {
                let s = table + i * 40;
                Some(Section {
                    size: u32_at(file, s + 8)?,
                    rva: u32_at(file, s + 12)?,
                    file_size: u32_at(file, s + 16)?,
                    file_offset: u32_at(file, s + 20)?,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(IconError::NotPe)?;
        let mut pe = Self {
            file,
            sections,
            root: 0,
        };
        pe.root = pe.offset(resource_rva).ok_or(IconError::NoResources)?;
        Ok(pe)
    }

    /// The file offset of `rva`.
    fn offset(&self, rva: u32) -> Option<usize> {
        self.sections.iter().find_map(|s| {
            let into = rva.checked_sub(s.rva)?;
            (into < s.size.max(s.file_size) && into < s.file_size)
                .then(|| (s.file_offset + into) as usize)
        })
    }

    /// A resource directory's entries: (id, or `None` for a named one;
    /// offset from the root; whether it is a subdirectory).
    fn entries(&self, dir: usize) -> Result<Vec<(Option<u32>, usize, bool)>, IconError> {
        let at = self.root + dir;
        let named = u16_at(self.file, at + 12).ok_or(IconError::Malformed)? as usize;
        let ids = u16_at(self.file, at + 14).ok_or(IconError::Malformed)? as usize;
        (0..named + ids)
            .map(|i| {
                let e = at + 16 + i * 8;
                let name = u32_at(self.file, e).ok_or(IconError::Malformed)?;
                let to = u32_at(self.file, e + 4).ok_or(IconError::Malformed)?;
                Ok((
                    (name & 0x8000_0000 == 0).then_some(name),
                    (to & 0x7FFF_FFFF) as usize,
                    to & 0x8000_0000 != 0,
                ))
            })
            .collect()
    }

    /// The first entry of a directory that is itself a directory, by id
    /// (`None`: any).
    fn subdirectory(&self, dir: usize, id: Option<u32>) -> Result<Option<usize>, IconError> {
        Ok(self
            .entries(dir)?
            .into_iter()
            .find(|&(i, _, sub)| sub && (id.is_none() || i == id))
            .map(|(_, to, _)| to))
    }

    /// The data of the first resource of `kind` (with `id`, or the first
    /// of that kind), in its first language.
    fn first_resource(&self, kind: u32, id: Option<u32>) -> Result<Option<&'a [u8]>, IconError> {
        let Some(names) = self.subdirectory(0, Some(kind))? else {
            return Ok(None);
        };
        let Some(languages) = self.subdirectory(names, id)? else {
            return Ok(None);
        };
        let Some(&(_, data, false)) = self.entries(languages)?.first() else {
            return Ok(None);
        };
        let at = self.root + data;
        let rva = u32_at(self.file, at).ok_or(IconError::Malformed)?;
        let size = u32_at(self.file, at + 4).ok_or(IconError::Malformed)? as usize;
        let start = self.offset(rva).ok_or(IconError::Malformed)?;
        self.file
            .get(start..start + size)
            .map(Some)
            .ok_or(IconError::Malformed)
    }
}

/// An icon group's entry: the image's id (its size and depth are read
/// from the image itself).
struct GroupEntry {
    id: u32,
}

fn group_entries(group: &[u8]) -> Result<Vec<GroupEntry>, IconError> {
    let count = u16_at(group, 4).ok_or(IconError::Malformed)? as usize;
    (0..count)
        .map(|i| {
            u16_at(group, 6 + i * 14 + 12)
                .map(|id| GroupEntry { id: u32::from(id) })
                .ok_or(IconError::Malformed)
        })
        .collect()
}

/// One icon image: a PNG, or a DIB with its mask.
fn decode_image(bytes: &[u8]) -> Option<IconImage> {
    if bytes.starts_with(PNG_SIGNATURE) {
        decode_png(bytes)
    } else {
        decode_dib(bytes)
    }
}

/// A DIB (`BITMAPINFOHEADER`, uncompressed, 1 to 32 bits a pixel) whose
/// height counts the colour rows and the mask's, both bottom up.
fn decode_dib(b: &[u8]) -> Option<IconImage> {
    let header = u32_at(b, 0)? as usize;
    let width = i32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
    let height = i32::from_le_bytes(b.get(8..12)?.try_into().ok()?) / 2;
    let bits = u32::from(u16_at(b, 14)?);
    let compression = u32_at(b, 16)?;
    let used = u32_at(b, 32)? as usize;
    if header < 40 || compression != 0 || !(1..=256).contains(&width) || height <= 0 {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let palette_len = match bits {
        1 | 4 | 8 => {
            if used == 0 {
                1 << bits
            } else {
                used
            }
        }
        24 | 32 => 0,
        _ => return None,
    };
    let palette = b.get(header..header + palette_len * 4)?;
    let pixels = header + palette_len * 4;
    let stride = (w * bits as usize).div_ceil(32) * 4;
    let mask_at = pixels + stride * h;
    let mask_stride = w.div_ceil(32) * 4;
    let mask = b.get(mask_at..mask_at + mask_stride * h);
    let colours = b.get(pixels..mask_at)?;
    let mut rgba = vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        // Rows are stored bottom up.
        let row = &colours[(h - 1 - y) * stride..][..stride];
        for x in 0..w {
            let (bgr, alpha) = match bits {
                32 => {
                    let p = &row[x * 4..x * 4 + 4];
                    ([p[0], p[1], p[2]], p[3])
                }
                24 => {
                    let p = &row[x * 3..x * 3 + 3];
                    ([p[0], p[1], p[2]], 255)
                }
                _ => {
                    let per_byte = 8 / bits as usize;
                    let byte = row[x / per_byte];
                    let shift = 8 - bits as usize * (x % per_byte + 1);
                    let index = ((byte >> shift) & ((1 << bits) - 1)) as usize;
                    let p = palette.get(index * 4..index * 4 + 3)?;
                    ([p[0], p[1], p[2]], 255)
                }
            };
            any_alpha |= bits == 32 && alpha != 0;
            let o = (y * w + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[bgr[2], bgr[1], bgr[0], alpha]);
        }
    }
    // Without an alpha channel of its own (or a 32-bit image whose alpha
    // is all zero, as older ones are) the mask says what shows: a set bit
    // is transparent.
    if !any_alpha {
        let mask = mask?;
        for y in 0..h {
            let row = &mask[(h - 1 - y) * mask_stride..][..mask_stride];
            for x in 0..w {
                let clear = row[x / 8] & (0x80 >> (x % 8)) != 0;
                rgba[(y * w + x) * 4 + 3] = if clear { 0 } else { 255 };
            }
        }
    }
    Some(IconImage {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A PNG of 8-bit RGB or RGBA, not interlaced (what icon images are).
fn decode_png(b: &[u8]) -> Option<IconImage> {
    let mut at = PNG_SIGNATURE.len();
    let mut header = None;
    let mut data = Vec::new();
    while at + 8 <= b.len() {
        let len = u32::from_be_bytes(b[at..at + 4].try_into().ok()?) as usize;
        let kind = &b[at + 4..at + 8];
        let body = b.get(at + 8..at + 8 + len)?;
        match kind {
            b"IHDR" => header = Some(body),
            b"IDAT" => data.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let h = header?;
    let width = u32::from_be_bytes(h.get(0..4)?.try_into().ok()?);
    let height = u32::from_be_bytes(h.get(4..8)?.try_into().ok()?);
    let (depth, colour, interlace) = (*h.get(8)?, *h.get(9)?, *h.get(12)?);
    let channels = match (depth, colour, interlace) {
        (8, 2, 0) => 3,
        (8, 6, 0) => 4,
        _ => return None,
    };
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let raw = decompress_to_vec_zlib(&data).ok()?;
    let stride = w * channels;
    if raw.len() < (stride + 1) * h {
        return None;
    }
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        let filter = raw[y * (stride + 1)];
        let line = &raw[y * (stride + 1) + 1..][..stride];
        for x in 0..stride {
            let a = if x >= channels {
                out[y * stride + x - channels]
            } else {
                0
            };
            let up = if y > 0 { out[(y - 1) * stride + x] } else { 0 };
            let c = if x >= channels && y > 0 {
                out[(y - 1) * stride + x - channels]
            } else {
                0
            };
            let predicted = match filter {
                0 => 0,
                1 => a,
                2 => up,
                3 => ((u16::from(a) + u16::from(up)) / 2) as u8,
                4 => paeth(a, up, c),
                _ => return None,
            };
            out[y * stride + x] = line[x].wrapping_add(predicted);
        }
    }
    let rgba = if channels == 4 {
        out
    } else {
        out.chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()
    };
    Some(IconImage {
        width,
        height,
        rgba,
    })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 32-bit DIB icon image: red, green / blue, clear (top row
    /// first), with its (unused) mask.
    fn dib32() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&4i32.to_le_bytes()); // colour + mask rows
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        // Bottom row first: blue, clear; then red, green. BGRA.
        b.extend_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0]);
        b.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 255]);
        b.extend_from_slice(&[0; 8]); // mask, 4 bytes a row
        b
    }

    /// A 2x1 4-bit DIB: palette black, white; pixels white, black; the
    /// mask clears the second.
    fn dib4() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&[0; 16]);
        b.extend_from_slice(&2u32.to_le_bytes()); // colours used
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]);
        b.extend_from_slice(&[0x10, 0, 0, 0]);
        b.extend_from_slice(&[0x40, 0, 0, 0]);
        b
    }

    /// A PE32 file with one section holding a resource tree of one icon
    /// group (id 1) listing the given images (ids 1, 2, ...).
    fn pe(images: &[Vec<u8>]) -> Vec<u8> {
        const SECTION_RVA: u32 = 0x1000;
        const SECTION_FILE: usize = 0x200;
        let mut f = vec![0u8; SECTION_FILE];
        f[0..2].copy_from_slice(b"MZ");
        f[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        f[0x40..0x44].copy_from_slice(b"PE\0\0");
        let coff = 0x44;
        f[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        f[coff + 16..coff + 18].copy_from_slice(&224u16.to_le_bytes());
        let opt = coff + 20;
        f[opt..opt + 2].copy_from_slice(&0x10Bu16.to_le_bytes());
        f[opt + 92..opt + 96].copy_from_slice(&16u32.to_le_bytes());
        f[opt + 96 + 16..opt + 96 + 20].copy_from_slice(&SECTION_RVA.to_le_bytes());

        // The resource tree, offsets from its root.
        let mut r = Vec::new();
        let dir = |r: &mut Vec<u8>, entries: &[(u32, u32)]| {
            r.extend_from_slice(&[0; 12]);
            r.extend_from_slice(&0u16.to_le_bytes());
            r.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            for &(id, to) in entries {
                r.extend_from_slice(&id.to_le_bytes());
                r.extend_from_slice(&to.to_le_bytes());
            }
        };
        let n = images.len() as u32;
        // Layout: root (2 types) | icon names (n) | group names (1) |
        // n+1 language dirs (1 each) | n+1 data entries | group | images.
        let sub = 0x8000_0000;
        let root_len = 16 + 2 * 8;
        let icon_names = root_len;
        let group_names = icon_names + 16 + n * 8;
        let langs = group_names + 16 + 8;
        let lang = |i: u32| langs + i * (16 + 8);
        let entries = lang(n + 1);
        let entry = |i: u32| entries + i * 16;
        let group_data = entry(n + 1);
        let group_len = 6 + 14 * n;
        dir(
            &mut r,
            &[
                (RT_ICON, icon_names | sub),
                (RT_GROUP_ICON, group_names | sub),
            ],
        );
        let icon_list: Vec<(u32, u32)> = (0..n).map(|i| (i + 1, lang(i) | sub)).collect();
        dir(&mut r, &icon_list);
        dir(&mut r, &[(1, lang(n) | sub)]);
        for i in 0..=n {
            dir(&mut r, &[(0x409, entry(i))]);
        }
        let mut data_at = group_data + group_len;
        let mut places = Vec::new();
        for image in images {
            places.push((data_at, image.len() as u32));
            data_at += image.len() as u32;
        }
        for (at, len) in &places {
            r.extend_from_slice(&(SECTION_RVA + at).to_le_bytes());
            r.extend_from_slice(&len.to_le_bytes());
            r.extend_from_slice(&[0; 8]);
        }
        r.extend_from_slice(&(SECTION_RVA + group_data).to_le_bytes());
        r.extend_from_slice(&group_len.to_le_bytes());
        r.extend_from_slice(&[0; 8]);
        r.extend_from_slice(&[0, 0, 1, 0]);
        r.extend_from_slice(&(n as u16).to_le_bytes());
        for i in 0..n {
            r.extend_from_slice(&[0; 12]);
            r.extend_from_slice(&(i as u16 + 1).to_le_bytes());
        }
        for image in images {
            r.extend_from_slice(image);
        }

        let s = opt + 224;
        f[s..s + 5].copy_from_slice(b".rsrc");
        f[s + 8..s + 12].copy_from_slice(&(r.len() as u32).to_le_bytes());
        f[s + 12..s + 16].copy_from_slice(&SECTION_RVA.to_le_bytes());
        f[s + 16..s + 20].copy_from_slice(&(r.len() as u32).to_le_bytes());
        f[s + 20..s + 24].copy_from_slice(&(SECTION_FILE as u32).to_le_bytes());
        f.extend_from_slice(&r);
        f
    }

    /// A 2x1 RGBA PNG, rows filtered Sub, of red then half-clear blue.
    fn png() -> Vec<u8> {
        let chunk = |out: &mut Vec<u8>, kind: &[u8], body: &[u8]| {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out.extend_from_slice(&[0; 4]); // CRC: not checked
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        // Sub: the second pixel is stored as its difference from the first.
        let raw = [
            1,
            255,
            0,
            0,
            255,
            0u8.wrapping_sub(255),
            0,
            255,
            128u8.wrapping_sub(255),
        ];
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let mut out = PNG_SIGNATURE.to_vec();
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// A 2x1 24-bit DIB (the depth of the installed game's icon): yellow,
    /// cyan; the mask clears the second.
    fn dib24() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        b.extend_from_slice(&[0, 255, 255, 255, 255, 0, 0, 0]); // BGR, padded to 8
        b.extend_from_slice(&[0x40, 0, 0, 0]);
        b
    }

    #[test]
    fn a_24_bit_image_is_cut_out_by_its_mask() {
        let images = program_icon(&pe(&[dib24()])).unwrap();
        assert_eq!(images[0].rgba, [255, 255, 0, 255, 0, 255, 255, 0]);
    }

    #[test]
    fn reads_an_ico_file() {
        let image = dib24();
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend_from_slice(&[2, 1, 0, 0, 1, 0, 24, 0]);
        ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&image);
        let images = ico_file(&ico).unwrap();
        assert_eq!(images[0].rgba, [255, 255, 0, 255, 0, 255, 255, 0]);
        assert_eq!(ico_file(b"MZ"), Err(IconError::NotIco));
    }

    #[test]
    fn reads_a_32_bit_image_through_the_resources() {
        let images = program_icon(&pe(&[dib32()])).unwrap();
        assert_eq!(images.len(), 1);
        let i = &images[0];
        assert_eq!((i.width, i.height), (2, 2));
        assert_eq!(
            i.rgba,
            [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0]
        );
    }

    #[test]
    fn a_paletted_image_is_cut_out_by_its_mask() {
        let images = program_icon(&pe(&[dib4()])).unwrap();
        assert_eq!(images[0].rgba, [255, 255, 255, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn reads_a_png_image() {
        let images = program_icon(&pe(&[dib4(), png()])).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!((images[1].width, images[1].height), (2, 1));
        assert_eq!(images[1].rgba, [255, 0, 0, 255, 0, 0, 255, 128]);
    }

    #[test]
    fn picks_the_largest_fitting_size() {
        let image = |w| IconImage {
            width: w,
            height: w,
            rgba: Vec::new(),
        };
        let all = [image(16), image(48), image(32)];
        assert_eq!(largest_up_to(&all, 32).unwrap().width, 32);
        assert_eq!(largest_up_to(&all, 256).unwrap().width, 48);
        assert_eq!(largest_up_to(&all, 8).unwrap().width, 16);
    }

    #[test]
    fn other_files_say_why() {
        assert_eq!(program_icon(b"not an exe"), Err(IconError::NotPe));
        let mut no_icon = pe(&[]);
        // The tree's types: rename both so neither is an icon type.
        let root = 0x200 + 16;
        no_icon[root..root + 4].copy_from_slice(&99u32.to_le_bytes());
        no_icon[root + 8..root + 12].copy_from_slice(&98u32.to_le_bytes());
        assert_eq!(program_icon(&no_icon), Err(IconError::NoIcon));
    }
}
