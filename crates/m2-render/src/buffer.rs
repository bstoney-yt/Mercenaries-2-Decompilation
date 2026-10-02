//! Vertex and index buffer descriptions and handles.
//!
//! Named after the engine's `PgVertexBufferPc` and `PgIndexBufferPc` classes
//! (from the executable's readable strings only; their code is not
//! available). The layout here is a clean-room design: a buffer is described
//! by a [`VertexLayout`] or [`IndexFormat`] plus a [`BufferUsage`], created
//! through a [`RenderBackend`](crate::RenderBackend), and referred to by a
//! generational handle afterwards.

use std::fmt;

/// What a vertex attribute means to the shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VertexSemantic {
    Position,
    Normal,
    Tangent,
    Binormal,
    Color,
    TexCoord(u8),
    BlendWeight,
    BlendIndices,
}

/// Storage format of one vertex attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VertexFormat {
    Float1,
    Float2,
    Float3,
    Float4,
    /// Four bytes, normalised to `0.0..=1.0` (D3D9 `D3DCOLOR` order is a
    /// backend concern).
    UNorm4x8,
    /// Four unsigned bytes, read as integers (blend indices).
    UInt4x8,
    Half2,
    Half4,
}

impl VertexFormat {
    /// Size in bytes.
    pub const fn size(self) -> u32 {
        match self {
            Self::Float1 | Self::UNorm4x8 | Self::UInt4x8 | Self::Half2 => 4,
            Self::Float2 | Self::Half4 => 8,
            Self::Float3 => 12,
            Self::Float4 => 16,
        }
    }
}

/// One attribute within a vertex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VertexElement {
    pub semantic: VertexSemantic,
    pub format: VertexFormat,
    /// Byte offset from the start of the vertex.
    pub offset: u32,
}

/// Errors from building a [`VertexLayout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("vertex layout has no elements")]
    Empty,
    #[error("element {index} ({semantic:?}) extends past the {stride}-byte stride")]
    OutOfStride {
        index: usize,
        semantic: VertexSemantic,
        stride: u32,
    },
    #[error("element {index} overlaps an earlier element")]
    Overlap { index: usize },
    #[error("semantic {0:?} appears more than once")]
    DuplicateSemantic(VertexSemantic),
}

/// The attributes of one vertex and the distance between vertices.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VertexLayout {
    elements: Vec<VertexElement>,
    stride: u32,
}

impl VertexLayout {
    /// Packs `attributes` in order with no padding.
    pub fn packed(attributes: &[(VertexSemantic, VertexFormat)]) -> Result<Self, LayoutError> {
        let mut offset = 0;
        let elements = attributes
            .iter()
            .map(|&(semantic, format)| {
                let element = VertexElement {
                    semantic,
                    format,
                    offset,
                };
                offset += format.size();
                element
            })
            .collect();
        Self::new(elements, offset)
    }

    /// Validates an explicit layout: every element fits in `stride`, none
    /// overlap, and no semantic repeats.
    pub fn new(elements: Vec<VertexElement>, stride: u32) -> Result<Self, LayoutError> {
        if elements.is_empty() {
            return Err(LayoutError::Empty);
        }
        for (index, e) in elements.iter().enumerate() {
            if e.offset + e.format.size() > stride {
                return Err(LayoutError::OutOfStride {
                    index,
                    semantic: e.semantic,
                    stride,
                });
            }
            for earlier in &elements[..index] {
                if earlier.semantic == e.semantic {
                    return Err(LayoutError::DuplicateSemantic(e.semantic));
                }
                let disjoint = e.offset >= earlier.offset + earlier.format.size()
                    || earlier.offset >= e.offset + e.format.size();
                if !disjoint {
                    return Err(LayoutError::Overlap { index });
                }
            }
        }
        Ok(Self { elements, stride })
    }

    pub fn elements(&self) -> &[VertexElement] {
        &self.elements
    }

    /// Bytes per vertex.
    pub fn stride(&self) -> u32 {
        self.stride
    }

    pub fn find(&self, semantic: VertexSemantic) -> Option<&VertexElement> {
        self.elements.iter().find(|e| e.semantic == semantic)
    }
}

/// Width of each index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndexFormat {
    U16,
    U32,
}

impl IndexFormat {
    pub const fn size(self) -> u32 {
        match self {
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }
}

/// How often the contents change, so a backend can pick the right memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BufferUsage {
    /// Written once at creation (level geometry, models).
    #[default]
    Static,
    /// Rewritten every frame or so (particles, UI, debug lines).
    Dynamic,
}

/// Everything needed to create a vertex buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexBufferDesc {
    pub layout: VertexLayout,
    pub vertex_count: u32,
    pub usage: BufferUsage,
}

impl VertexBufferDesc {
    pub fn size_in_bytes(&self) -> u64 {
        u64::from(self.layout.stride()) * u64::from(self.vertex_count)
    }
}

/// Everything needed to create an index buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexBufferDesc {
    pub format: IndexFormat,
    pub index_count: u32,
    pub usage: BufferUsage,
}

impl IndexBufferDesc {
    pub fn size_in_bytes(&self) -> u64 {
        u64::from(self.format.size()) * u64::from(self.index_count)
    }
}

/// A slot index plus a generation counter, so a handle to a destroyed buffer
/// is detected instead of silently aliasing whatever reused its slot.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RawHandle {
    pub index: u32,
    pub generation: u32,
}

impl fmt::Debug for RawHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.index, self.generation)
    }
}

macro_rules! handle {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub RawHandle);
    };
}

handle!(
    /// A vertex buffer owned by a backend.
    VertexBufferHandle
);
handle!(
    /// An index buffer owned by a backend.
    IndexBufferHandle
);

#[cfg(test)]
mod tests {
    use super::*;
    use VertexFormat::*;
    use VertexSemantic::*;

    #[test]
    fn packed_layout_offsets_and_stride() {
        let layout =
            VertexLayout::packed(&[(Position, Float3), (Normal, Float3), (TexCoord(0), Float2)])
                .unwrap();
        assert_eq!(layout.stride(), 32);
        assert_eq!(layout.find(Normal).unwrap().offset, 12);
        assert_eq!(layout.find(TexCoord(0)).unwrap().offset, 24);
        assert!(layout.find(Color).is_none());
    }

    #[test]
    fn rejects_bad_layouts() {
        assert_eq!(VertexLayout::new(vec![], 16), Err(LayoutError::Empty));

        let pos = VertexElement {
            semantic: Position,
            format: Float3,
            offset: 0,
        };
        assert!(matches!(
            VertexLayout::new(vec![pos], 8),
            Err(LayoutError::OutOfStride { index: 0, .. })
        ));

        let overlapping = VertexElement {
            semantic: Normal,
            format: Float3,
            offset: 8,
        };
        assert_eq!(
            VertexLayout::new(vec![pos, overlapping], 32),
            Err(LayoutError::Overlap { index: 1 })
        );

        let again = VertexElement { offset: 12, ..pos };
        assert_eq!(
            VertexLayout::new(vec![pos, again], 32),
            Err(LayoutError::DuplicateSemantic(Position))
        );
    }

    #[test]
    fn padded_layout_is_allowed() {
        let layout = VertexLayout::new(
            vec![
                VertexElement {
                    semantic: Position,
                    format: Float3,
                    offset: 0,
                },
                VertexElement {
                    semantic: Color,
                    format: UNorm4x8,
                    offset: 16,
                },
            ],
            20,
        )
        .unwrap();
        assert_eq!(layout.stride(), 20);
    }

    #[test]
    fn buffer_sizes() {
        let vb = VertexBufferDesc {
            layout: VertexLayout::packed(&[(Position, Float3)]).unwrap(),
            vertex_count: 100,
            usage: BufferUsage::Static,
        };
        assert_eq!(vb.size_in_bytes(), 1200);
        let ib = IndexBufferDesc {
            format: IndexFormat::U16,
            index_count: 300,
            usage: BufferUsage::Dynamic,
        };
        assert_eq!(ib.size_in_bytes(), 600);
    }
}
