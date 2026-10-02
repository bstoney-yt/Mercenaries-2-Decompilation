//! Vertex declarations and vertex streams as stored in model assets.
//!
//! A model's vertex stream is described by a list of Direct3D 9
//! `D3DVERTEXELEMENT9` records, 8 bytes each:
//!
//! ```text
//! 0x00  u16  stream
//! 0x02  u16  byte offset within the vertex
//! 0x04  u8   D3DDECLTYPE
//! 0x05  u8   D3DDECLMETHOD (always 0, default)
//! 0x06  u8   D3DDECLUSAGE
//! 0x07  u8   usage index
//! ```
//!
//! closed by the `D3DDECL_END()` record `{0xFF, 0, UNUSED, 0, 0, 0}`. The
//! declaration is plain Direct3D 9 data, so the values are the public
//! `D3DDECLTYPE` and `D3DDECLUSAGE` constants.
//!
//! [`VertexDecl::layout`] turns one stream of a declaration into an
//! [`m2_render::VertexLayout`], so the stream bytes can be uploaded as they
//! are; [`VertexStream`] reads attributes back as `f32` for tools and tests.

use m2_render::buffer::LayoutError;
use m2_render::{VertexElement, VertexFormat, VertexLayout, VertexSemantic};

use crate::bytes::{OutOfBounds, Reader};

/// Length of one declaration record.
pub const ELEMENT_LEN: usize = 8;
/// Stream number of the `D3DDECL_END()` record.
pub const END_STREAM: u16 = 0xFF;
/// Most records a declaration may hold, as in Direct3D 9 (`MAXD3DDECLLENGTH`
/// plus the end record).
pub const MAX_ELEMENTS: usize = 65;

/// `D3DDECLTYPE` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DeclType {
    Float1 = 0,
    Float2 = 1,
    Float3 = 2,
    Float4 = 3,
    D3dColor = 4,
    UByte4 = 5,
    Short2 = 6,
    Short4 = 7,
    UByte4N = 8,
    Short2N = 9,
    Short4N = 10,
    UShort2N = 11,
    UShort4N = 12,
    UDec3 = 13,
    Dec3N = 14,
    Float16x2 = 15,
    Float16x4 = 16,
    Unused = 17,
}

impl DeclType {
    pub fn from_u8(v: u8) -> Option<Self> {
        use DeclType::*;
        const ALL: [DeclType; 18] = [
            Float1, Float2, Float3, Float4, D3dColor, UByte4, Short2, Short4, UByte4N, Short2N,
            Short4N, UShort2N, UShort4N, UDec3, Dec3N, Float16x2, Float16x4, Unused,
        ];
        ALL.get(usize::from(v)).copied()
    }

    /// Size in bytes.
    pub const fn size(self) -> u32 {
        use DeclType::*;
        match self {
            Unused => 0,
            Float1 | D3dColor | UByte4 | Short2 | UByte4N | Short2N | UShort2N | UDec3 | Dec3N
            | Float16x2 => 4,
            Float2 | Short4 | Short4N | UShort4N | Float16x4 => 8,
            Float3 => 12,
            Float4 => 16,
        }
    }

    /// The matching render format, where the renderer has one.
    pub fn render_format(self) -> Option<VertexFormat> {
        use DeclType::*;
        Some(match self {
            Float1 => VertexFormat::Float1,
            Float2 => VertexFormat::Float2,
            Float3 => VertexFormat::Float3,
            Float4 => VertexFormat::Float4,
            D3dColor | UByte4N => VertexFormat::UNorm4x8,
            UByte4 => VertexFormat::UInt4x8,
            Float16x2 => VertexFormat::Half2,
            Float16x4 => VertexFormat::Half4,
            _ => return None,
        })
    }
}

/// `D3DDECLUSAGE` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DeclUsage {
    Position = 0,
    BlendWeight = 1,
    BlendIndices = 2,
    Normal = 3,
    PSize = 4,
    TexCoord = 5,
    Tangent = 6,
    Binormal = 7,
    TessFactor = 8,
    PositionT = 9,
    Color = 10,
    Fog = 11,
    Depth = 12,
    Sample = 13,
}

impl DeclUsage {
    pub fn from_u8(v: u8) -> Option<Self> {
        use DeclUsage::*;
        const ALL: [DeclUsage; 14] = [
            Position,
            BlendWeight,
            BlendIndices,
            Normal,
            PSize,
            TexCoord,
            Tangent,
            Binormal,
            TessFactor,
            PositionT,
            Color,
            Fog,
            Depth,
            Sample,
        ];
        ALL.get(usize::from(v)).copied()
    }
}

/// Errors from parsing a declaration or reading a stream.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VertexError {
    #[error("declaration is {0:#x} bytes, not a whole number of records")]
    DeclLength(usize),
    #[error("declaration has more than {MAX_ELEMENTS} records")]
    TooManyElements,
    #[error("declaration has no end record")]
    MissingEnd,
    #[error("declaration has records after its end record")]
    TrailingElements,
    #[error("element {index} has unknown type {value}")]
    UnknownType { index: usize, value: u8 },
    #[error("element {index} has unknown usage {value}")]
    UnknownUsage { index: usize, value: u8 },
    #[error("element {index} uses tessellation method {value}")]
    UnsupportedMethod { index: usize, value: u8 },
    #[error("element {index} ({usage:?}) has no renderer equivalent for {ty:?}")]
    UnsupportedType {
        index: usize,
        usage: DeclUsage,
        ty: DeclType,
    },
    #[error("element {index} has usage {usage:?}{usage_index}, which the renderer cannot express")]
    UnsupportedUsage {
        index: usize,
        usage: DeclUsage,
        usage_index: u8,
    },
    #[error("stream {stream} layout: {source}")]
    Layout { stream: u16, source: LayoutError },
    #[error("stream {0} has no elements")]
    NoSuchStream(u16),
    #[error("{vertices} vertices of {stride} bytes need {needed:#x} bytes, got {got:#x}")]
    StreamLength {
        vertices: u32,
        stride: u32,
        needed: u64,
        got: usize,
    },
}

impl From<OutOfBounds> for VertexError {
    fn from(e: OutOfBounds) -> Self {
        Self::DeclLength(e.offset + e.len)
    }
}

/// One `D3DVERTEXELEMENT9`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeclElement {
    pub stream: u16,
    pub offset: u16,
    pub ty: DeclType,
    pub usage: DeclUsage,
    pub usage_index: u8,
}

/// A vertex declaration without its end record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VertexDecl {
    elements: Vec<DeclElement>,
}

impl VertexDecl {
    /// Parses records up to and including the end record, which must be the
    /// last record in `data`.
    pub fn parse(data: &[u8]) -> Result<Self, VertexError> {
        if !data.len().is_multiple_of(ELEMENT_LEN) {
            return Err(VertexError::DeclLength(data.len()));
        }
        let count = data.len() / ELEMENT_LEN;
        if count > MAX_ELEMENTS {
            return Err(VertexError::TooManyElements);
        }
        let mut r = Reader::at(data, 0);
        let mut elements = Vec::with_capacity(count.saturating_sub(1));
        for index in 0..count {
            let stream = r.u16()?;
            let offset = r.u16()?;
            let [ty, method, usage, usage_index] = r.array()?;
            if stream == END_STREAM {
                if index + 1 != count {
                    return Err(VertexError::TrailingElements);
                }
                return Ok(Self { elements });
            }
            let ty = DeclType::from_u8(ty).ok_or(VertexError::UnknownType { index, value: ty })?;
            if method != 0 {
                return Err(VertexError::UnsupportedMethod {
                    index,
                    value: method,
                });
            }
            let usage = DeclUsage::from_u8(usage).ok_or(VertexError::UnknownUsage {
                index,
                value: usage,
            })?;
            elements.push(DeclElement {
                stream,
                offset,
                ty,
                usage,
                usage_index,
            });
        }
        Err(VertexError::MissingEnd)
    }

    pub fn elements(&self) -> &[DeclElement] {
        &self.elements
    }

    /// The streams the declaration uses, ascending.
    pub fn streams(&self) -> Vec<u16> {
        let mut s: Vec<u16> = self.elements.iter().map(|e| e.stream).collect();
        s.sort_unstable();
        s.dedup();
        s
    }

    /// Finds an element by usage and usage index.
    pub fn find(&self, usage: DeclUsage, usage_index: u8) -> Option<&DeclElement> {
        self.elements
            .iter()
            .find(|e| e.usage == usage && e.usage_index == usage_index)
    }

    /// The renderer layout of one stream, with vertices `stride` bytes apart.
    pub fn layout(&self, stream: u16, stride: u32) -> Result<VertexLayout, VertexError> {
        let mut elements = Vec::new();
        for (index, e) in self.elements.iter().enumerate() {
            if e.stream != stream {
                continue;
            }
            let unsupported_usage = VertexError::UnsupportedUsage {
                index,
                usage: e.usage,
                usage_index: e.usage_index,
            };
            let semantic = match (e.usage, e.usage_index) {
                (DeclUsage::Position, 0) => VertexSemantic::Position,
                (DeclUsage::Normal, 0) => VertexSemantic::Normal,
                (DeclUsage::Tangent, 0) => VertexSemantic::Tangent,
                (DeclUsage::Binormal, 0) => VertexSemantic::Binormal,
                (DeclUsage::Color, 0) => VertexSemantic::Color,
                (DeclUsage::BlendWeight, 0) => VertexSemantic::BlendWeight,
                (DeclUsage::BlendIndices, 0) => VertexSemantic::BlendIndices,
                (DeclUsage::TexCoord, i) => VertexSemantic::TexCoord(i),
                _ => return Err(unsupported_usage),
            };
            let format = e.ty.render_format().ok_or(VertexError::UnsupportedType {
                index,
                usage: e.usage,
                ty: e.ty,
            })?;
            elements.push(VertexElement {
                semantic,
                format,
                offset: u32::from(e.offset),
            });
        }
        if elements.is_empty() {
            return Err(VertexError::NoSuchStream(stream));
        }
        VertexLayout::new(elements, stride).map_err(|source| VertexError::Layout { stream, source })
    }
}

/// The vertices of one stream: `count` records of `stride` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexStream<'a> {
    pub stride: u32,
    pub count: u32,
    pub data: &'a [u8],
}

impl<'a> VertexStream<'a> {
    /// Checks `data` holds exactly `count` vertices.
    pub fn new(data: &'a [u8], stride: u32, count: u32) -> Result<Self, VertexError> {
        let needed = u64::from(stride) * u64::from(count);
        if needed != data.len() as u64 {
            return Err(VertexError::StreamLength {
                vertices: count,
                stride,
                needed,
                got: data.len(),
            });
        }
        Ok(Self {
            stride,
            count,
            data,
        })
    }

    /// Reads `element` from every vertex as four floats. Missing components
    /// read as 0 (and `w` as 1), as Direct3D 9 expands them; byte formats
    /// read as their integer or normalised value.
    ///
    /// Returns `None` if the element does not fit in the stride.
    pub fn read(&self, element: &DeclElement) -> Option<Vec<[f32; 4]>> {
        let offset = usize::from(element.offset);
        let size = element.ty.size() as usize;
        if offset + size > self.stride as usize {
            return None;
        }
        Some(
            self.data
                .chunks_exact(self.stride as usize)
                .map(|v| decode(element.ty, &v[offset..offset + size]))
                .collect(),
        )
    }
}

/// Decodes one attribute value.
fn decode(ty: DeclType, b: &[u8]) -> [f32; 4] {
    let f32_at = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let i16_at = |i: usize| i16::from_le_bytes([b[i], b[i + 1]]);
    let snorm = |v: i16| (f32::from(v) / 32767.0).max(-1.0);
    let dec3 = |v: u32, signed: bool| {
        let field = |shift: u32| {
            let x = (v >> shift) & 0x3FF;
            if signed {
                // Sign-extend the 10-bit field.
                let s = ((x << 22) as i32) >> 22;
                (s as f32 / 511.0).max(-1.0)
            } else {
                x as f32
            }
        };
        [field(0), field(10), field(20), 1.0]
    };
    use DeclType::*;
    match ty {
        Float1 => [f32_at(0), 0.0, 0.0, 1.0],
        Float2 => [f32_at(0), f32_at(4), 0.0, 1.0],
        Float3 => [f32_at(0), f32_at(4), f32_at(8), 1.0],
        Float4 => [f32_at(0), f32_at(4), f32_at(8), f32_at(12)],
        // Stored B, G, R, A; returned R, G, B, A.
        D3dColor => [b[2], b[1], b[0], b[3]].map(|c| f32::from(c) / 255.0),
        UByte4 => [b[0], b[1], b[2], b[3]].map(f32::from),
        UByte4N => [b[0], b[1], b[2], b[3]].map(|c| f32::from(c) / 255.0),
        Short2 => [f32::from(i16_at(0)), f32::from(i16_at(2)), 0.0, 1.0],
        Short4 => [0, 2, 4, 6].map(|i| f32::from(i16_at(i))),
        Short2N => [snorm(i16_at(0)), snorm(i16_at(2)), 0.0, 1.0],
        Short4N => [0, 2, 4, 6].map(|i| snorm(i16_at(i))),
        UShort2N => [
            f32::from(u16_at(0)) / 65535.0,
            f32::from(u16_at(2)) / 65535.0,
            0.0,
            1.0,
        ],
        UShort4N => [0, 2, 4, 6].map(|i| f32::from(u16_at(i)) / 65535.0),
        UDec3 => dec3(u32::from_le_bytes([b[0], b[1], b[2], b[3]]), false),
        Dec3N => dec3(u32::from_le_bytes([b[0], b[1], b[2], b[3]]), true),
        Float16x2 => [half_to_f32(u16_at(0)), half_to_f32(u16_at(2)), 0.0, 1.0],
        Float16x4 => [0, 2, 4, 6].map(|i| half_to_f32(u16_at(i))),
        Unused => [0.0, 0.0, 0.0, 1.0],
    }
}

/// Converts an IEEE 754 binary16 value.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1F);
    let mant = u32::from(h & 0x3FF);
    let bits = match (exp, mant) {
        (0, 0) => sign,
        (0, m) => {
            // Subnormal: value is m * 2^-24.
            let v = m as f32 * f32::from_bits(0x3380_0000);
            return if sign != 0 { -v } else { v };
        }
        (0x1F, m) => sign | 0x7F80_0000 | (m << 13),
        (e, m) => sign | ((e + 112) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Encodes declaration records followed by the end record.
    pub fn decl(elements: &[(u16, u16, DeclType, DeclUsage, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        for &(stream, offset, ty, usage, index) in elements {
            out.extend_from_slice(&stream.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&[ty as u8, 0, usage as u8, index]);
        }
        out.extend_from_slice(&[0xFF, 0, 0, 0, 17, 0, 0, 0]);
        out
    }

    /// Encodes a binary16 value; exact for the test values used.
    pub fn f32_to_half(v: f32) -> u16 {
        let bits = v.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        if v == 0.0 {
            return sign;
        }
        let exp = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
        assert!((1..31).contains(&exp), "{v} is not a normal half");
        sign | ((exp as u16) << 10) | ((bits >> 13) & 0x3FF) as u16
    }

    #[test]
    fn half_conversion() {
        assert_eq!(half_to_f32(0x3C00), 1.0);
        assert_eq!(half_to_f32(0xC000), -2.0);
        assert_eq!(half_to_f32(0x7BFF), 65504.0);
        assert_eq!(half_to_f32(0x0001), 2f32.powi(-24));
        assert_eq!(half_to_f32(0x8000).to_bits(), (-0.0f32).to_bits());
        assert!(half_to_f32(0x7C00).is_infinite());
        assert!(half_to_f32(0x7E00).is_nan());
        for v in [0.5, -1.25, 1024.0, 0.099975586] {
            assert_eq!(half_to_f32(f32_to_half(v)), v);
        }
    }

    #[test]
    fn parses_mesh_declaration_and_layout() {
        use DeclType::*;
        use DeclUsage::*;
        let bytes = decl(&[
            (0, 0, Float16x4, Position, 0),
            (0, 8, Float16x2, TexCoord, 0),
            (0, 12, Float16x4, Normal, 0),
        ]);
        let d = VertexDecl::parse(&bytes).unwrap();
        assert_eq!(d.elements().len(), 3);
        assert_eq!(d.streams(), vec![0]);
        let layout = d.layout(0, 0x14).unwrap();
        assert_eq!(layout.stride(), 0x14);
        assert_eq!(
            layout.find(VertexSemantic::TexCoord(0)).unwrap().format,
            VertexFormat::Half2
        );
        assert_eq!(d.layout(1, 0x14), Err(VertexError::NoSuchStream(1)));
        assert!(matches!(
            d.layout(0, 0x10),
            Err(VertexError::Layout { stream: 0, .. })
        ));
    }

    #[test]
    fn rejects_bad_declarations() {
        use DeclType::*;
        use DeclUsage::*;
        let good = decl(&[(0, 0, Float3, Position, 0)]);
        assert_eq!(VertexDecl::parse(&good[..8]), Err(VertexError::MissingEnd));
        assert_eq!(
            VertexDecl::parse(&good[..7]),
            Err(VertexError::DeclLength(7))
        );
        let mut trailing = good.clone();
        trailing.extend_from_slice(&good[..8]);
        assert_eq!(
            VertexDecl::parse(&trailing),
            Err(VertexError::TrailingElements)
        );
        let mut bad_type = good.clone();
        bad_type[4] = 18;
        assert_eq!(
            VertexDecl::parse(&bad_type),
            Err(VertexError::UnknownType {
                index: 0,
                value: 18
            })
        );
        let mut bad_usage = good.clone();
        bad_usage[6] = 14;
        assert_eq!(
            VertexDecl::parse(&bad_usage),
            Err(VertexError::UnknownUsage {
                index: 0,
                value: 14
            })
        );
        let mut tess = good;
        tess[5] = 1;
        assert!(matches!(
            VertexDecl::parse(&tess),
            Err(VertexError::UnsupportedMethod { .. })
        ));
        let too_many = vec![0u8; ELEMENT_LEN * (MAX_ELEMENTS + 1)];
        assert_eq!(
            VertexDecl::parse(&too_many),
            Err(VertexError::TooManyElements)
        );
    }

    #[test]
    fn layout_rejects_inexpressible_elements() {
        use DeclType::*;
        use DeclUsage::*;
        let d = VertexDecl::parse(&decl(&[(0, 0, Short4N, Position, 0)])).unwrap();
        assert!(matches!(
            d.layout(0, 8),
            Err(VertexError::UnsupportedType { .. })
        ));
        let d = VertexDecl::parse(&decl(&[(0, 0, Float3, Normal, 1)])).unwrap();
        assert!(matches!(
            d.layout(0, 12),
            Err(VertexError::UnsupportedUsage { .. })
        ));
    }

    #[test]
    fn reads_stream_attributes() {
        use DeclType::*;
        use DeclUsage::*;
        let d = VertexDecl::parse(&decl(&[
            (0, 0, Float16x4, Position, 0),
            (0, 8, D3dColor, Color, 0),
            (0, 12, UByte4, BlendIndices, 0),
        ]))
        .unwrap();
        let mut data = Vec::new();
        for v in [[1.0, -2.0, 0.5, 1.0], [4.0, 8.0, -16.0, 1.0]] {
            for c in v {
                data.extend_from_slice(&f32_to_half(c).to_le_bytes());
            }
            data.extend_from_slice(&[0, 0x80, 0xFF, 0xFF]);
            data.extend_from_slice(&[1, 2, 3, 4]);
        }
        let s = VertexStream::new(&data, 16, 2).unwrap();
        let pos = s.read(d.find(Position, 0).unwrap()).unwrap();
        assert_eq!(pos, vec![[1.0, -2.0, 0.5, 1.0], [4.0, 8.0, -16.0, 1.0]]);
        let col = s.read(d.find(Color, 0).unwrap()).unwrap();
        assert_eq!(col[0], [1.0, 128.0 / 255.0, 0.0, 1.0]);
        let idx = s.read(d.find(BlendIndices, 0).unwrap()).unwrap();
        assert_eq!(idx[1], [1.0, 2.0, 3.0, 4.0]);

        assert!(matches!(
            VertexStream::new(&data, 16, 3),
            Err(VertexError::StreamLength { .. })
        ));
        let narrow = VertexStream::new(&data, 8, 4).unwrap();
        assert_eq!(narrow.read(d.find(Color, 0).unwrap()), None);
    }

    #[test]
    fn decodes_packed_formats() {
        assert_eq!(
            decode(DeclType::Short2N, &[0xFF, 0x7F, 0x00, 0x80]),
            [1.0, -1.0, 0.0, 1.0]
        );
        // x = 511, y = -511, z = 0.
        let v: u32 = 0x1FF | (0x201 << 10);
        assert_eq!(
            decode(DeclType::Dec3N, &v.to_le_bytes()),
            [1.0, -1.0, 0.0, 1.0]
        );
        assert_eq!(
            decode(DeclType::UDec3, &v.to_le_bytes()),
            [511.0, 513.0, 0.0, 1.0]
        );
    }
}
