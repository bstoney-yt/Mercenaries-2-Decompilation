//! Models on the GPU.
//!
//! [`GpuModel::upload`] turns a parsed [`Model`] into vertex and index
//! buffers on any [`RenderBackend`], one pair per primitive group, with its
//! materials registered through a shared [`MaterialCache`] so textures used
//! by many models are uploaded once. [`GpuModel::draw`] queues every piece
//! at a world transform.
//!
//! Skinned geometry is drawn in its bind pose for now; animation drives it
//! once the animation runtime lands.

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::Model;
use m2_assets::model::TOPOLOGY;
use m2_math::glam::{Mat4, Vec3};
use m2_render::{
    BackendError, BlendMode, DrawCall, DrawQueue, Geometry, IndexBufferHandle, Layer, MaterialDesc,
    MaterialId, Primitive, RenderBackend, TextureDesc, TextureFormat, TextureHandle,
    VertexBufferHandle, generate_mips,
};

use crate::assets::{AssetSource, DecodedTexture};

/// Textures and materials shared between models.
#[derive(Debug)]
pub struct MaterialCache {
    textures: HashMap<u32, Option<(TextureHandle, bool)>>,
    materials: HashMap<(u32, Option<u32>), MaterialId>,
    defined: Vec<(MaterialId, MaterialDesc)>,
    next_material: u32,
    /// Largest texture width uploaded; bigger mips are skipped.
    pub max_texture_width: u32,
}

impl Default for MaterialCache {
    fn default() -> Self {
        Self {
            textures: HashMap::new(),
            materials: HashMap::new(),
            defined: Vec::new(),
            // 0 stays the backend's default material.
            next_material: 1,
            max_texture_width: 2048,
        }
    }
}

impl MaterialCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Texture `hash` on the GPU, uploading it on first use. `None` (cached)
    /// when the texture can't be read; the material then draws untextured.
    pub fn texture<R: Read + Seek>(
        &mut self,
        backend: &mut dyn RenderBackend,
        source: &mut AssetSource<R>,
        hash: u32,
    ) -> Result<Option<(TextureHandle, bool)>, BackendError> {
        if let Some(t) = self.textures.get(&hash) {
            return Ok(*t);
        }
        let uploaded = match source.texture(hash, self.max_texture_width) {
            Ok(t) => Some(upload_texture(backend, &t)?),
            Err(_) => None,
        };
        self.textures.insert(hash, uploaded);
        Ok(uploaded)
    }

    /// The material for a model material record: its first texture as base
    /// colour, alpha-tested when that texture has alpha.
    pub fn material<R: Read + Seek>(
        &mut self,
        backend: &mut dyn RenderBackend,
        source: &mut AssetSource<R>,
        material: &m2_assets::model::Material,
    ) -> Result<MaterialId, BackendError> {
        let texture_hash = material.textures.first().copied().filter(|&t| t != 0);
        let key = (material.name_hash, texture_hash);
        if let Some(&id) = self.materials.get(&key) {
            return Ok(id);
        }
        let texture = match texture_hash {
            Some(t) => self.texture(backend, source, t)?,
            None => None,
        };
        let desc = MaterialDesc {
            texture: texture.map(|t| t.0),
            blend: match texture {
                Some((_, true)) => BlendMode::Masked { cutoff: 0.5 },
                _ => BlendMode::Opaque,
            },
            color: if texture.is_some() {
                [1.0; 4]
            } else {
                [0.6, 0.6, 0.6, 1.0]
            },
            ..MaterialDesc::default()
        };
        let id = self.define(backend, &desc)?;
        self.materials.insert(key, id);
        Ok(id)
    }

    /// Registers a material that belongs to no model (ground, debug shapes).
    pub fn define(
        &mut self,
        backend: &mut dyn RenderBackend,
        desc: &MaterialDesc,
    ) -> Result<MaterialId, BackendError> {
        let id = MaterialId(self.next_material);
        self.next_material += 1;
        backend.set_material(id, desc)?;
        self.defined.push((id, *desc));
        Ok(id)
    }

    /// Redefines every material drawing both faces (`true`) or as defined
    /// (`false`); a debugging aid for checking winding.
    pub fn set_double_sided(
        &self,
        backend: &mut dyn RenderBackend,
        on: bool,
    ) -> Result<(), BackendError> {
        for (id, desc) in &self.defined {
            let desc = MaterialDesc {
                double_sided: on || desc.double_sided,
                ..*desc
            };
            backend.set_material(*id, &desc)?;
        }
        Ok(())
    }

    /// Materials defined so far.
    pub fn len(&self) -> usize {
        self.defined.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defined.is_empty()
    }
}

/// Uploads a decoded texture, generating any mips its chain lacks.
pub fn upload_texture(
    backend: &mut dyn RenderBackend,
    t: &DecodedTexture,
) -> Result<(TextureHandle, bool), BackendError> {
    let desc = TextureDesc {
        width: t.width,
        height: t.height,
        format: TextureFormat::Rgba8Srgb,
    };
    let generated;
    let mips: Vec<&[u8]> = if t.mips.len() as u32 == desc.full_chain_len() {
        t.mips.iter().map(Vec::as_slice).collect()
    } else {
        generated = generate_mips(t.width, t.height, &t.mips[0]);
        generated.iter().map(Vec::as_slice).collect()
    };
    Ok((backend.create_texture(&desc, &mips)?, t.alpha))
}

/// One primitive group on the GPU.
#[derive(Debug)]
struct Piece {
    vertex_buffer: VertexBufferHandle,
    index_buffer: IndexBufferHandle,
    /// Node to model.
    local: Mat4,
    /// (first index, index count, material).
    ranges: Vec<(u32, u32, MaterialId)>,
    centre: Vec3,
}

/// A model's geometry at one level of detail, ready to draw.
#[derive(Debug)]
pub struct GpuModel {
    pub hash: u32,
    pieces: Vec<Piece>,
    /// Model-space bounds of everything drawn.
    pub bounds: (Vec3, Vec3),
    /// Groups skipped because their vertex format isn't supported.
    pub skipped_groups: usize,
}

impl GpuModel {
    /// Uploads `model`'s geometry at `lod` (0 is the finest).
    pub fn upload<R: Read + Seek>(
        backend: &mut dyn RenderBackend,
        source: &mut AssetSource<R>,
        materials: &mut MaterialCache,
        hash: u32,
        model: &Model<'_>,
        lod: u32,
    ) -> Result<Self, BackendError> {
        let mut ids = Vec::with_capacity(model.materials.len());
        for m in &model.materials {
            ids.push(materials.material(backend, source, m)?);
        }
        let mut pieces = Vec::new();
        let mut skipped_groups = 0;
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for (segment, geometry) in model.lod(lod) {
            let local = model.node_to_model(segment.node).unwrap_or(Mat4::IDENTITY);
            for group in &geometry.groups {
                let Ok(desc) = group.vertex_buffer_desc() else {
                    skipped_groups += 1;
                    continue;
                };
                let Some(vertices) = group.vertices.data.get(..desc.size_in_bytes() as usize)
                else {
                    skipped_groups += 1;
                    continue;
                };
                let vertex_buffer = backend.create_vertex_buffer(&desc, Some(vertices))?;
                let index_buffer = backend
                    .create_index_buffer(&group.index_buffer_desc(), Some(&group.index_bytes()))?;
                let ranges = group
                    .material_ranges
                    .iter()
                    .filter(|r| r.start_index + r.index_count() <= group.indices.len() as u32)
                    .map(|r| {
                        let id = ids.get(r.material as usize).copied().unwrap_or_default();
                        (r.start_index, r.index_count(), id)
                    })
                    .collect();
                let mut centre = Vec3::ZERO;
                if let Some(pos) = group
                    .decl
                    .find(m2_assets::vertex::DeclUsage::Position, 0)
                    .and_then(|e| group.vertices.read(e))
                {
                    let (mut plo, mut phi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                    for p in &pos {
                        let v = local.transform_point3(Vec3::new(p[0], p[1], p[2]));
                        plo = plo.min(v);
                        phi = phi.max(v);
                    }
                    if !pos.is_empty() {
                        lo = lo.min(plo);
                        hi = hi.max(phi);
                        centre = (plo + phi) * 0.5;
                    }
                }
                pieces.push(Piece {
                    vertex_buffer,
                    index_buffer,
                    local,
                    ranges,
                    centre,
                });
            }
        }
        if pieces.is_empty() {
            lo = Vec3::ZERO;
            hi = Vec3::ZERO;
        }
        Ok(Self {
            hash,
            pieces,
            bounds: (lo, hi),
            skipped_groups,
        })
    }

    /// Queues every piece at `world`.
    pub fn draw(&self, queue: &mut DrawQueue, world: Mat4) {
        for piece in &self.pieces {
            let transform = world * piece.local;
            let origin = world.transform_point3(piece.centre);
            for &(first, count, material) in &piece.ranges {
                queue.push(Primitive {
                    call: DrawCall {
                        material,
                        topology: TOPOLOGY,
                        vertex_buffer: piece.vertex_buffer,
                        geometry: Geometry::Indexed {
                            buffer: piece.index_buffer,
                            first,
                            count,
                            base_vertex: 0,
                        },
                        world: transform,
                    },
                    layer: Layer::WORLD,
                    translucent: false,
                    sort_origin: origin,
                    user: 0,
                });
            }
        }
    }

    /// Frees the model's buffers. Textures stay in the [`MaterialCache`].
    pub fn destroy(self, backend: &mut dyn RenderBackend) -> Result<(), BackendError> {
        for p in self.pieces {
            backend.destroy_vertex_buffer(p.vertex_buffer)?;
            backend.destroy_index_buffer(p.index_buffer)?;
        }
        Ok(())
    }

    pub fn draw_count(&self) -> usize {
        self.pieces.iter().map(|p| p.ranges.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_render::NullBackend;

    #[test]
    fn materials_get_fresh_ids_and_toggle_sides() {
        let mut backend = NullBackend::new();
        let mut cache = MaterialCache::new();
        let a = cache
            .define(&mut backend, &MaterialDesc::default())
            .unwrap();
        let b = cache
            .define(&mut backend, &MaterialDesc::default())
            .unwrap();
        assert_ne!(a, b);
        assert_ne!(a, MaterialId(0));
        cache.set_double_sided(&mut backend, true).unwrap();
        assert!(backend.material(b).double_sided);
        cache.set_double_sided(&mut backend, false).unwrap();
        assert!(!backend.material(b).double_sided);
    }

    #[test]
    fn partial_mip_chains_are_completed() {
        let mut backend = NullBackend::new();
        let t = DecodedTexture {
            width: 4,
            height: 4,
            mips: vec![vec![255; 64]],
            alpha: true,
        };
        let (handle, alpha) = upload_texture(&mut backend, &t).unwrap();
        assert!(alpha);
        assert_eq!(backend.texture_desc(handle).unwrap().width, 4);
        assert_eq!(backend.live_buffers().texture, 1);
    }
}
