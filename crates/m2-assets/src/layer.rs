//! Entity layers: the tables that place everything in the world, from the
//! terrain cells to buildings, props, AI and spawners.
//!
//! A layer (asset type hash `e6b81a54`, in `layers_static` and the
//! `vz_state_*` blocks) is a `UCFX` container of components, each a table
//! of per-entity records described by its own schema:
//!
//! ```text
//! CHDR  u32 x 2 (0x00380000, 2)
//! enum  named enumerations shared by the schemas (layout not decoded)
//! COMP/ one per component:
//!       info  cstr name, u32 component hash, u32 unknown, u32 record count,
//!             u32 unknown
//!       schm  u32 field count, u32 record size (unpacked), then 16 bytes
//!             per field: u32 type, u32 name hash, u32 unknown,
//!             u16 offset (unpacked), u8 bit shift, u8 bit width
//!       data  per entity: u32 entity id, then the record
//! flgt  u32
//! flgs  per-entity flags (layout not decoded)
//! ```
//!
//! In the data a record is usually exactly the schema's record size. Two
//! exceptions are known: string fields (type 8) are stored inline as
//! NUL-terminated text, and `Transform` packs its eight bitfields (type 4,
//! one bit width each) into a single `u16`, so its records are 38 bytes:
//!
//! ```text
//! Transform  f32 x 4 position (w unused), f32 x 4 rotation (x, y, z, w),
//!            u32 unknown, u16 packed bitfields
//! Name       cstr name, u8 unknown
//! ```
//!
//! Components that name an asset hold its name hash in their first word,
//! for example `TerrainObject` (a terrain cell), `LowResTerrainObject` (a
//! low-resolution tile) and `ModelName` (a model).
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 923 layers in the archives of a
//! legally installed copy (about 101,000 entities, 72 component types);
//! every fixed-size component's data is a whole number of records. The
//! component and field names are the strings stored in the files. No bytes
//! from the archives are committed.

use std::collections::HashMap;

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of entity layers.
pub const LAYER_TYPE: u32 = 0xe6b8_1a54;
/// Schema field type of inline strings.
pub const STRING_FIELD: u32 = 8;

const FIELD_LEN: usize = 16;
/// Stored size of a `Transform` record after the entity id.
const TRANSFORM_LEN: usize = 38;

/// Errors from parsing a layer.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LayerError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("missing {0} section")]
    Missing(&'static str),
    #[error("component {0} has a malformed name")]
    BadName(usize),
    #[error("component {name}: schema is {len:#x} bytes for {fields} fields")]
    Schema {
        name: String,
        len: usize,
        fields: u32,
    },
}

impl From<OutOfBounds> for LayerError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One schema field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub ty: u32,
    pub name_hash: u32,
    pub unk_0x08: u32,
    /// Offset in the unpacked record.
    pub offset: u16,
    pub bit_shift: u8,
    pub bit_width: u8,
}

/// One component table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component<'a> {
    pub name: &'a str,
    pub hash: u32,
    pub unk_info_0x04: u32,
    pub count: u32,
    pub unk_info_0x0c: u32,
    /// Unpacked record size from the schema.
    pub record_size: u32,
    pub fields: Vec<Field>,
    pub data: &'a [u8],
}

impl<'a> Component<'a> {
    /// `(entity, record)` pairs, if every record has the same stored size
    /// (the data divides evenly by the count and no field is a string).
    pub fn records(&self) -> Option<Vec<(u32, &'a [u8])>> {
        let n = self.count as usize;
        if n == 0 || self.data.len() % n != 0 || self.fields.iter().any(|f| f.ty == STRING_FIELD) {
            return None;
        }
        let len = self.data.len() / n;
        if len < 4 {
            return None;
        }
        Some(
            self.data
                .chunks_exact(len)
                .map(|r| (u32::from_le_bytes([r[0], r[1], r[2], r[3]]), &r[4..]))
                .collect(),
        )
    }
}

/// A placement from a `Transform` record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: [f32; 3],
    /// (x, y, z, w).
    pub rotation: [f32; 4],
    pub unk_0x20: u32,
    pub flags: u16,
}

/// A parsed layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer<'a> {
    pub header: [u32; 2],
    /// The `enum` section, undecoded.
    pub enums: &'a [u8],
    pub components: Vec<Component<'a>>,
    pub flgt: u32,
    /// The `flgs` section, undecoded.
    pub flags: &'a [u8],
}

impl<'a> Layer<'a> {
    pub fn parse(asset: &'a [u8]) -> Result<Self, LayerError> {
        let u = Ucfx::parse(asset)?;
        let top = |cc: [u8; 4], name| u.top_level(cc).ok_or(LayerError::Missing(name));
        let mut r = Reader::at(top(*b"CHDR", "CHDR")?, 0);
        let header = [r.u32()?, r.u32()?];
        let enums = top(*b"enum", "enum")?;
        let flgt = Reader::at(top(*b"flgt", "flgt")?, 0).u32()?;
        let flags = top(*b"flgs", "flgs")?;

        let mut components = Vec::new();
        for (index, c) in u
            .children(None)
            .filter(|&i| &u.sections()[i].fourcc == b"COMP")
            .enumerate()
        {
            let part = |cc: &[u8; 4], name| {
                u.children(Some(c))
                    .map(|i| &u.sections()[i])
                    .find(|s| &s.fourcc == cc)
                    .and_then(|s| s.data)
                    .ok_or(LayerError::Missing(name))
            };
            let info = part(b"info", "COMP/info")?;
            let mut r = Reader::at(info, 0);
            let name = std::str::from_utf8(r.cstr()?).map_err(|_| LayerError::BadName(index))?;
            let (hash, unk_info_0x04, count, unk_info_0x0c) =
                (r.u32()?, r.u32()?, r.u32()?, r.u32()?);

            let schm = part(b"schm", "COMP/schm")?;
            let mut r = Reader::at(schm, 0);
            let (nfields, record_size) = (r.u32()?, r.u32()?);
            if schm.len() as u64 != 8 + u64::from(nfields) * FIELD_LEN as u64 {
                return Err(LayerError::Schema {
                    name: name.to_owned(),
                    len: schm.len(),
                    fields: nfields,
                });
            }
            let fields = (0..nfields)
                .map(|_| {
                    let (ty, name_hash, unk_0x08) = (r.u32()?, r.u32()?, r.u32()?);
                    let [offset @ .., bit_shift, bit_width]: [u8; 4] = r.array()?;
                    Ok(Field {
                        ty,
                        name_hash,
                        unk_0x08,
                        offset: u16::from_le_bytes(offset),
                        bit_shift,
                        bit_width,
                    })
                })
                .collect::<Result<_, OutOfBounds>>()?;
            components.push(Component {
                name,
                hash,
                unk_info_0x04,
                count,
                unk_info_0x0c,
                record_size,
                fields,
                data: part(b"data", "COMP/data")?,
            });
        }
        Ok(Self {
            header,
            enums,
            components,
            flgt,
            flags,
        })
    }

    pub fn component(&self, name: &str) -> Option<&Component<'a>> {
        self.components.iter().find(|c| c.name == name)
    }

    /// Entity names from the `Name` component.
    pub fn names(&self) -> HashMap<u32, &'a str> {
        let mut out = HashMap::new();
        let Some(c) = self.component("Name") else {
            return out;
        };
        let mut r = Reader::at(c.data, 0);
        for _ in 0..c.count {
            let Ok(entity) = r.u32() else { break };
            let Ok(name) = r.cstr() else { break };
            if r.take(1).is_err() {
                break;
            }
            if let Ok(name) = std::str::from_utf8(name) {
                out.insert(entity, name);
            }
        }
        out
    }

    /// Entity placements from the `Transform` component.
    pub fn placements(&self) -> HashMap<u32, Placement> {
        let Some(c) = self.component("Transform") else {
            return HashMap::new();
        };
        if c.data.len() != c.count as usize * (4 + TRANSFORM_LEN) {
            return HashMap::new();
        }
        c.data
            .chunks_exact(4 + TRANSFORM_LEN)
            .map(|r| {
                let f = |i: usize| f32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]);
                let entity = u32::from_le_bytes([r[0], r[1], r[2], r[3]]);
                (
                    entity,
                    Placement {
                        position: [f(4), f(8), f(12)],
                        rotation: [f(20), f(24), f(28), f(32)],
                        unk_0x20: u32::from_le_bytes([r[36], r[37], r[38], r[39]]),
                        flags: u16::from_le_bytes([r[40], r[41]]),
                    },
                )
            })
            .collect()
    }

    /// The first `u32` of every record of a component, by entity: the asset
    /// hash for components such as `TerrainObject` and `ModelName`.
    pub fn hashes(&self, component: &str) -> HashMap<u32, u32> {
        self.component(component)
            .and_then(Component::records)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, r)| r.len() >= 4)
            .map(|(e, r)| (e, u32::from_le_bytes([r[0], r[1], r[2], r[3]])))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn comp(
        name: &str,
        hash: u32,
        count: u32,
        fields: &[(u32, u32, u32)],
        size: u32,
        data: Vec<u8>,
    ) -> UcfxNode {
        let mut info = name.as_bytes().to_vec();
        info.push(0);
        info.extend(le(&[hash, 1, count, 0]));
        let mut schm = le(&[fields.len() as u32, size]);
        for &(ty, h, packed) in fields {
            schm.extend(le(&[ty, h, 0, packed]));
        }
        UcfxNode::Container(
            *b"COMP",
            vec![
                UcfxNode::Leaf(*b"info", info),
                UcfxNode::Leaf(*b"schm", schm),
                UcfxNode::Leaf(*b"data", data),
            ],
        )
    }

    fn layer() -> Vec<u8> {
        let mut names = Vec::new();
        for (e, n) in [(10u32, "Terrain_r00_c00"), (11, "Terrain_r00_c01")] {
            names.extend(e.to_le_bytes());
            names.extend(n.as_bytes());
            names.extend([0, 0]);
        }
        let mut transforms = Vec::new();
        for (e, x) in [(10u32, -3800.0f32), (11, -3400.0)] {
            transforms.extend(e.to_le_bytes());
            for v in [x, 5.0, -3800.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
                transforms.extend(v.to_le_bytes());
            }
            transforms.extend(le(&[0x8000_6911]));
            transforms.extend(0u16.to_le_bytes());
        }
        ucfx(&[
            UcfxNode::Leaf(*b"CHDR", le(&[0x0038_0000, 2])),
            UcfxNode::Leaf(*b"enum", le(&[0])),
            comp(
                "Name",
                0x1de5_c824,
                2,
                &[(8, 0x1de5_c824, 0), (1, 0x12af_a0b8, 4)],
                5,
                names,
            ),
            comp(
                "TerrainObject",
                0x6c82_ebe5,
                2,
                &[(6, 0xc47b_227c, 0)],
                4,
                le(&[10, 0xAAAA, 11, 0xBBBB]),
            ),
            comp(
                "Transform",
                0x753e_b623,
                2,
                &[
                    (0xb, 0x753e_b623, 0),
                    (5, 0x7945_0675, 0x20),
                    (4, 0xeb18_87dc, 0x0400_0024),
                ],
                0x34,
                transforms,
            ),
            UcfxNode::Leaf(*b"flgt", le(&[0xe9da_bb4a])),
            UcfxNode::Leaf(*b"flgs", le(&[2])),
        ])
    }

    #[test]
    fn parses_components() {
        let a = layer();
        let l = Layer::parse(&a).unwrap();
        assert_eq!(l.header, [0x0038_0000, 2]);
        assert_eq!(l.components.len(), 3);
        let t = l.component("Transform").unwrap();
        assert_eq!(t.record_size, 0x34);
        assert_eq!(t.fields[2].offset, 0x24);
        assert_eq!((t.fields[2].bit_shift, t.fields[2].bit_width), (0, 4));
        assert_eq!(l.names()[&11], "Terrain_r00_c01");
        assert_eq!(
            l.hashes("TerrainObject"),
            HashMap::from([(10, 0xAAAA), (11, 0xBBBB)])
        );
        let p = l.placements()[&11];
        assert_eq!(p.position, [-3400.0, 5.0, -3800.0]);
        assert_eq!(p.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(p.unk_0x20, 0x8000_6911);
        // Strings make Name variable-size.
        assert!(l.component("Name").unwrap().records().is_none());
        assert!(l.hashes("Missing").is_empty());
    }

    #[test]
    fn rejects_bad_schemas() {
        let a = layer();
        // Claim one more field than the schema holds.
        let at = a
            .windows(8)
            .position(|w| w == le(&[1, 4]).as_slice())
            .unwrap();
        let mut b = a.clone();
        b[at] = 2;
        assert!(matches!(Layer::parse(&b), Err(LayerError::Schema { .. })));
    }

    #[test]
    fn byte_flips_never_panic() {
        let a = layer();
        for at in 0..a.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut b = a.clone();
                b[at] = v;
                if let Ok(l) = Layer::parse(&b) {
                    let _ = (l.names(), l.placements(), l.hashes("TerrainObject"));
                    for c in &l.components {
                        let _ = c.records();
                    }
                }
            }
        }
    }
}
