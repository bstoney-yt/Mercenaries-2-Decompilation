//! Models on the GPU.
//!
//! Loading is split so the slow half can run on a loader thread:
//! [`CpuModel::build`] turns a parsed [`Model`] into upload-ready vertex and
//! index bytes (no GPU, `Send`), and [`GpuModel::upload`] creates the
//! buffers on a [`RenderBackend`] and resolves materials through a shared
//! [`MaterialCache`], where each texture is uploaded once however many
//! models use it. [`GpuModel::draw`] queues every piece at a world
//! transform.
//!
//! Skinned geometry is drawn in its bind pose for now; animation drives it
//! once the animation runtime lands.

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::model::{GeometryKind, Material, TOPOLOGY};
use m2_assets::vertex::DeclUsage;
use m2_assets::{Model, Ucfx};
use m2_math::glam::{Mat4, Vec3};
use m2_render::{
    BackendError, BlendMode, DrawCall, DrawQueue, Geometry, IndexBufferDesc, IndexBufferHandle,
    Layer, MaterialDesc, MaterialId, Primitive, RenderBackend, SkinHandle, TextureDesc,
    TextureFormat, TextureHandle, VertexBufferDesc, VertexBufferHandle, generate_mips,
};

use crate::assets::{AssetError, AssetSource, DecodedTexture, ModelData};
use crate::physics::CollisionShape;

/// Textures and materials shared between models.
#[derive(Debug)]
pub struct MaterialCache {
    /// `None`: the texture could not be read; its materials draw untextured.
    textures: HashMap<u32, Option<(TextureHandle, bool)>>,
    materials: HashMap<(u32, Option<u32>), MaterialId>,
    defined: Vec<(MaterialId, MaterialDesc)>,
    next_material: u32,
    /// Every material, including ones defined later, draws both faces.
    all_double_sided: bool,
    /// Largest texture width to read for models; bigger mips are skipped.
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
            all_double_sided: false,
            max_texture_width: 1024,
        }
    }
}

impl MaterialCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether texture `hash` has been added (successfully or not).
    pub fn has_texture(&self, hash: u32) -> bool {
        self.textures.contains_key(&hash)
    }

    /// The uploaded texture `hash` and whether it has alpha.
    pub fn texture(&self, hash: u32) -> Option<(TextureHandle, bool)> {
        self.textures.get(&hash).copied().flatten()
    }

    /// Uploads a decoded texture under `hash`, or records that it could not
    /// be read (`None`). Does nothing if `hash` is already known.
    pub fn add_texture(
        &mut self,
        backend: &mut dyn RenderBackend,
        hash: u32,
        texture: Option<&DecodedTexture>,
    ) -> Result<(), BackendError> {
        if self.has_texture(hash) {
            return Ok(());
        }
        let uploaded = match texture {
            Some(t) if !t.mips.is_empty() => Some(upload_texture(backend, t)?),
            _ => None,
        };
        self.textures.insert(hash, uploaded);
        Ok(())
    }

    /// Reads and uploads the textures in `hashes` not yet known.
    pub fn load_textures<R: Read + Seek>(
        &mut self,
        backend: &mut dyn RenderBackend,
        source: &mut AssetSource<R>,
        hashes: impl IntoIterator<Item = u32>,
    ) -> Result<(), BackendError> {
        for hash in hashes {
            if !self.has_texture(hash) {
                let t = source.texture(hash, self.max_texture_width).ok();
                self.add_texture(backend, hash, t.as_ref())?;
            }
        }
        Ok(())
    }

    /// The material for a model material record: its first texture as base
    /// colour, alpha-tested when that texture has alpha. Add the texture
    /// first; a texture not yet added draws untextured and is not cached.
    pub fn material(
        &mut self,
        backend: &mut dyn RenderBackend,
        material: &Material,
    ) -> Result<MaterialId, BackendError> {
        let texture_hash = base_texture(material);
        let key = (material.name_hash, texture_hash);
        if let Some(&id) = self.materials.get(&key) {
            return Ok(id);
        }
        let texture = texture_hash.and_then(|t| self.texture(t));
        let desc = MaterialDesc {
            texture: texture.map(|t| t.0),
            blend: match texture {
                Some((_, true)) => BlendMode::Masked { cutoff: 0.5 },
                _ => BlendMode::Opaque,
            },
            double_sided: matches!(texture, Some((_, true))),
            color: if texture.is_some() {
                [1.0; 4]
            } else {
                [0.6, 0.6, 0.6, 1.0]
            },
            ..MaterialDesc::default()
        };
        let id = self.define(backend, &desc)?;
        if texture_hash.is_none_or(|t| self.has_texture(t)) {
            self.materials.insert(key, id);
        }
        Ok(id)
    }

    /// Registers a material that belongs to no model (terrain, ground,
    /// debug shapes).
    pub fn define(
        &mut self,
        backend: &mut dyn RenderBackend,
        desc: &MaterialDesc,
    ) -> Result<MaterialId, BackendError> {
        let id = MaterialId(self.next_material);
        self.next_material += 1;
        backend.set_material(
            id,
            &MaterialDesc {
                double_sided: self.all_double_sided || desc.double_sided,
                ..*desc
            },
        )?;
        self.defined.push((id, *desc));
        Ok(id)
    }

    /// Redefines every material, and defines later ones, drawing both faces
    /// (`true`) or as defined (`false`); a debugging aid for checking winding.
    pub fn set_double_sided(
        &mut self,
        backend: &mut dyn RenderBackend,
        on: bool,
    ) -> Result<(), BackendError> {
        self.all_double_sided = on;
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

/// A material record's base-colour texture (slot 0).
pub fn base_texture(material: &Material) -> Option<u32> {
    material.textures.first().copied().filter(|&t| t != 0)
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

/// One primitive group, ready to upload.
#[derive(Clone, Debug)]
pub struct CpuPiece {
    pub vertex_desc: VertexBufferDesc,
    pub vertices: Vec<u8>,
    pub index_desc: IndexBufferDesc,
    pub indices: Vec<u8>,
    /// Geometry to model space.
    pub local: Mat4,
    /// (first index, index count, index into the model's materials).
    pub ranges: Vec<(u32, u32, usize)>,
    /// Model-space centre, for sorting.
    pub centre: Vec3,
    /// For skinned geometry, the node each blend index names; empty for
    /// rigid geometry.
    pub bones: Vec<u16>,
    /// For rigid geometry, the node it hangs from (`local` is that node's
    /// rest transform), so posed models can move it with the node.
    pub node: Option<u16>,
}

/// A model's geometry at one level of detail, decoded but not uploaded.
#[derive(Clone, Debug)]
pub struct CpuModel {
    pub hash: u32,
    pub pieces: Vec<CpuPiece>,
    pub materials: Vec<Material>,
    /// Model-space bounds of everything drawn.
    pub bounds: (Vec3, Vec3),
    /// Groups left out because their vertex format isn't supported.
    pub skipped_groups: usize,
    /// Static collision in model space; [`from_data`](Self::from_data)
    /// builds it, [`build`](Self::build) does not.
    pub collision: Option<CollisionShape>,
}

impl CpuModel {
    /// The geometry of `model` at the finest level of detail at or after
    /// `lod` that is present (streamed levels may be missing).
    pub fn build(hash: u32, model: &Model<'_>, lod: u32) -> Self {
        let lod = (lod..model.info.lod_count.max(lod + 1))
            .find(|&l| model.has_lod(l))
            .unwrap_or(lod);
        let mut pieces = Vec::new();
        let mut skipped_groups = 0;
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for (segment, geometry) in model.lod(lod) {
            // Skinned positions are already in model space.
            let skinned = geometry.kind == GeometryKind::Skin;
            let local = match geometry.kind {
                GeometryKind::Skin => Mat4::IDENTITY,
                _ => model.node_to_model(segment.node).unwrap_or(Mat4::IDENTITY),
            };
            for group in &geometry.groups {
                let Ok(vertex_desc) = group.vertex_buffer_desc() else {
                    skipped_groups += 1;
                    continue;
                };
                let Some(vertices) = group
                    .vertices
                    .data
                    .get(..vertex_desc.size_in_bytes() as usize)
                else {
                    skipped_groups += 1;
                    continue;
                };
                let ranges = group
                    .material_ranges
                    .iter()
                    .filter(|r| r.start_index + r.index_count() <= group.indices.len() as u32)
                    .map(|r| (r.start_index, r.index_count(), r.material as usize))
                    .collect();
                let mut centre = Vec3::ZERO;
                if let Some(pos) = group
                    .decl
                    .find(DeclUsage::Position, 0)
                    .and_then(|e| group.vertices.read(e))
                    .filter(|p| !p.is_empty())
                {
                    let (mut plo, mut phi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                    for p in &pos {
                        let v = local.transform_point3(Vec3::new(p[0], p[1], p[2]));
                        plo = plo.min(v);
                        phi = phi.max(v);
                    }
                    lo = lo.min(plo);
                    hi = hi.max(phi);
                    centre = (plo + phi) * 0.5;
                }
                pieces.push(CpuPiece {
                    vertex_desc,
                    vertices: vertices.to_vec(),
                    index_desc: group.index_buffer_desc(),
                    indices: group.index_bytes(),
                    local,
                    ranges,
                    centre,
                    bones: if skinned {
                        group.info.bone_nodes()
                    } else {
                        Vec::new()
                    },
                    node: (!skinned).then_some(segment.node),
                });
            }
        }
        if lo.x > hi.x {
            (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
        }
        Self {
            hash,
            pieces,
            materials: model.materials.clone(),
            bounds: (lo, hi),
            skipped_groups,
            collision: None,
        }
    }

    /// Parses `data` and builds its geometry at `lod`, with its collision.
    pub fn from_data(data: &ModelData, lod: u32) -> Result<Self, AssetError> {
        let model = data.parse()?;
        let mut cpu = Self::build(data.hash, &model, lod);
        let phy2 = Ucfx::parse(&data.base)
            .ok()
            .and_then(|u| u.top_level(*b"PHY2"));
        cpu.collision = CollisionShape::for_model(&model, phy2, lod);
        Ok(cpu)
    }

    /// Base-colour textures the model's materials use.
    pub fn texture_hashes(&self) -> impl Iterator<Item = u32> + '_ {
        self.materials.iter().filter_map(base_texture)
    }
}

/// One primitive group on the GPU.
#[derive(Debug)]
struct Piece {
    vertex_buffer: VertexBufferHandle,
    index_buffer: IndexBufferHandle,
    local: Mat4,
    ranges: Vec<(u32, u32, MaterialId)>,
    centre: Vec3,
    bones: Vec<u16>,
    node: Option<u16>,
}

/// A model's geometry on the GPU, ready to draw.
#[derive(Debug)]
pub struct GpuModel {
    pub hash: u32,
    pieces: Vec<Piece>,
    /// Model-space bounds of everything drawn.
    pub bounds: (Vec3, Vec3),
    /// Groups left out because their vertex format isn't supported.
    pub skipped_groups: usize,
}

impl GpuModel {
    /// Uploads `cpu`. Its textures should already be in `materials`.
    pub fn upload(
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        cpu: &CpuModel,
    ) -> Result<Self, BackendError> {
        let mut ids = Vec::with_capacity(cpu.materials.len());
        for m in &cpu.materials {
            ids.push(materials.material(backend, m)?);
        }
        let mut pieces = Vec::with_capacity(cpu.pieces.len());
        for p in &cpu.pieces {
            let vertex_buffer = backend.create_vertex_buffer(&p.vertex_desc, Some(&p.vertices))?;
            let index_buffer = backend.create_index_buffer(&p.index_desc, Some(&p.indices))?;
            pieces.push(Piece {
                vertex_buffer,
                index_buffer,
                local: p.local,
                ranges: p
                    .ranges
                    .iter()
                    .map(|&(first, count, m)| {
                        (first, count, ids.get(m).copied().unwrap_or_default())
                    })
                    .collect(),
                // Kept in geometry space, so any transform places it.
                centre: p.local.inverse().transform_point3(p.centre),
                bones: p.bones.clone(),
                node: p.node,
            });
        }
        Ok(Self {
            hash: cpu.hash,
            pieces,
            bounds: cpu.bounds,
            skipped_groups: cpu.skipped_groups,
        })
    }

    /// Reads, builds and uploads model `hash` and its textures in one go
    /// (on the calling thread).
    pub fn load<R: Read + Seek>(
        backend: &mut dyn RenderBackend,
        source: &mut AssetSource<R>,
        materials: &mut MaterialCache,
        hash: u32,
        lod: u32,
    ) -> Result<Self, crate::AssetError> {
        let data = source.model(hash)?;
        let cpu = CpuModel::build(hash, &data.parse()?, lod);
        materials.load_textures(backend, source, cpu.texture_hashes().collect::<Vec<_>>())?;
        Ok(Self::upload(backend, materials, &cpu)?)
    }

    /// Queues every piece at `world`, skinned geometry in its bind pose.
    pub fn draw(&self, queue: &mut DrawQueue, world: Mat4) {
        for piece in &self.pieces {
            Self::push(queue, piece, world, None);
        }
    }

    /// Whether any piece is skinned.
    pub fn is_skinned(&self) -> bool {
        self.pieces.iter().any(|p| !p.bones.is_empty())
    }

    /// Queues every piece at `world`, posed: rigid pieces follow their
    /// node's posed matrix in `nodes` (as
    /// [`Skeleton::model_matrices`](crate::Skeleton::model_matrices)
    /// returns) and skinned ones blend `skin` (as
    /// [`Skeleton::skin_matrices`](crate::Skeleton::skin_matrices)). Palettes
    /// are set on `backend`, so call this inside a frame.
    pub fn draw_posed(
        &self,
        backend: &mut dyn RenderBackend,
        queue: &mut DrawQueue,
        world: Mat4,
        nodes: &[Mat4],
        skin: &[Mat4],
    ) -> Result<(), BackendError> {
        let mut palette = Vec::new();
        for piece in &self.pieces {
            let handle = if piece.bones.is_empty() {
                if let Some(&m) = piece.node.and_then(|n| nodes.get(usize::from(n))) {
                    Self::push_at(queue, piece, world * m, None);
                    continue;
                }
                None
            } else {
                palette.clear();
                palette.extend(
                    piece
                        .bones
                        .iter()
                        .map(|&n| skin.get(usize::from(n)).copied().unwrap_or(Mat4::IDENTITY)),
                );
                Some(backend.set_skin(&palette)?)
            };
            Self::push(queue, piece, world, handle);
        }
        Ok(())
    }

    fn push(queue: &mut DrawQueue, piece: &Piece, world: Mat4, skin: Option<SkinHandle>) {
        Self::push_at(queue, piece, world * piece.local, skin);
    }

    /// Queues `piece` with geometry-to-world `transform`.
    fn push_at(queue: &mut DrawQueue, piece: &Piece, transform: Mat4, skin: Option<SkinHandle>) {
        let origin = transform.transform_point3(piece.centre);
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
                    skin,
                    instances: None,
                },
                layer: Layer::WORLD,
                translucent: false,
                sort_origin: origin,
                user: 0,
            });
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

    fn material(name_hash: u32, texture: u32) -> Material {
        Material {
            name_hash,
            constants: Default::default(),
            flags: 0,
            textures: vec![texture],
            technique: 0,
            unk_tail: 0,
        }
    }

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

    #[test]
    fn materials_follow_their_textures() {
        let mut backend = NullBackend::new();
        let mut cache = MaterialCache::new();
        let opaque = DecodedTexture {
            width: 1,
            height: 1,
            mips: vec![vec![1, 2, 3, 255]],
            alpha: false,
        };
        let cutout = DecodedTexture {
            alpha: true,
            ..opaque.clone()
        };
        cache.add_texture(&mut backend, 10, Some(&opaque)).unwrap();
        cache.add_texture(&mut backend, 11, Some(&cutout)).unwrap();
        cache.add_texture(&mut backend, 12, None).unwrap();

        let m = cache.material(&mut backend, &material(1, 10)).unwrap();
        assert_eq!(backend.material(m).blend, BlendMode::Opaque);
        assert!(backend.material(m).texture.is_some());
        // The same record again is the same material.
        assert_eq!(cache.material(&mut backend, &material(1, 10)).unwrap(), m);

        let m = cache.material(&mut backend, &material(2, 11)).unwrap();
        assert_eq!(backend.material(m).blend, BlendMode::Masked { cutoff: 0.5 });

        let m = cache.material(&mut backend, &material(3, 12)).unwrap();
        assert!(backend.material(m).texture.is_none());

        // A texture not added yet: untextured now, retried later.
        let early = cache.material(&mut backend, &material(4, 13)).unwrap();
        cache.add_texture(&mut backend, 13, Some(&opaque)).unwrap();
        let late = cache.material(&mut backend, &material(4, 13)).unwrap();
        assert_ne!(early, late);
        assert!(backend.material(late).texture.is_some());
        assert_eq!(backend.live_buffers().texture, 3);
    }
}
