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

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};

use m2_assets::model::{GeometryKind, Material, TOPOLOGY};
use m2_assets::vertex::DeclUsage;
use m2_assets::{Model, Ucfx};
use m2_math::glam::{Mat4, Quat, Vec3};
use m2_render::{
    BackendError, BlendMode, DrawCall, DrawQueue, Geometry, IndexBufferDesc, IndexBufferHandle,
    Layer, MaterialDesc, MaterialId, NormalMap, Primitive, RenderBackend, SkinHandle, Specular,
    TextureDesc, TextureFormat, TextureHandle, VertexBufferDesc, VertexBufferHandle, generate_mips,
};

use crate::assets::{AssetError, AssetSource, DecodedTexture, ModelData};
use crate::physics::CollisionShape;
use crate::technique::Technique;

/// An uploaded texture and what its pixels say about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Uploaded {
    pub handle: TextureHandle,
    /// Its alpha cuts shapes out.
    pub alpha: bool,
    /// As a normal map, x is in alpha rather than red.
    pub alpha_x: bool,
}

/// Textures and materials shared between models.
#[derive(Debug)]
pub struct MaterialCache {
    /// `None`: the texture could not be read; its materials draw untextured.
    textures: HashMap<u32, Option<Uploaded>>,
    /// By name, base texture, technique and depth bias.
    materials: HashMap<(u32, Option<u32>, u32, bool), MaterialId>,
    defined: Vec<(MaterialId, MaterialDesc)>,
    /// Materials that blend over what is behind them.
    translucent: HashSet<MaterialId>,
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
            translucent: HashSet::new(),
            // 0 stays the backend's default material.
            next_material: 1,
            all_double_sided: false,
            max_texture_width: 2048,
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
        self.uploaded(hash).map(|t| (t.handle, t.alpha))
    }

    /// The uploaded texture `hash`.
    pub fn uploaded(&self, hash: u32) -> Option<Uploaded> {
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
            Some(t) if !t.mips.is_empty() => {
                let (handle, alpha) = upload_texture(backend, t)?;
                Some(Uploaded {
                    handle,
                    alpha,
                    alpha_x: alpha_holds_x(t),
                })
            }
            _ => None,
        };
        self.textures.insert(hash, uploaded);
        Ok(())
    }

    /// Reads and uploads the textures in `requests` not yet known: (hash,
    /// whether it holds data rather than colour), as
    /// [`material_textures`] lists them.
    pub fn load_textures<R: Read + Seek>(
        &mut self,
        backend: &mut dyn RenderBackend,
        source: &mut AssetSource<R>,
        requests: impl IntoIterator<Item = (u32, bool)>,
    ) -> Result<(), BackendError> {
        for (hash, linear) in requests {
            if !self.has_texture(hash) {
                let t = source
                    .texture(hash, self.max_texture_width)
                    .ok()
                    .map(|mut t| {
                        t.linear = linear;
                        t
                    });
                self.add_texture(backend, hash, t.as_ref())?;
            }
        }
        Ok(())
    }

    /// The material for a model material record, shaded as its technique
    /// says (see [`Technique`]): base colour from slot 0, alpha-tested when
    /// that texture has alpha, and the specular and normal maps when the
    /// technique has them. Add the textures first; a texture not yet added
    /// is left out and the material is not cached.
    pub fn material(
        &mut self,
        backend: &mut dyn RenderBackend,
        material: &Material,
    ) -> Result<MaterialId, BackendError> {
        self.material_biased(backend, material, false)
    }

    /// As [`material`](Self::material), for a surface laid over another
    /// (roads and markings on the ground): it wins the depth test.
    pub fn overlay_material(
        &mut self,
        backend: &mut dyn RenderBackend,
        material: &Material,
    ) -> Result<MaterialId, BackendError> {
        self.material_biased(backend, material, true)
    }

    fn material_biased(
        &mut self,
        backend: &mut dyn RenderBackend,
        material: &Material,
        depth_bias: bool,
    ) -> Result<MaterialId, BackendError> {
        let texture_hash = base_texture(material);
        let key = (
            material.name_hash,
            texture_hash,
            material.technique,
            depth_bias,
        );
        if let Some(&id) = self.materials.get(&key) {
            return Ok(id);
        }
        let technique = Technique::from_hash(material.technique);
        let slot = |i: usize| material.textures.get(i).copied().filter(|&t| t != 0);
        let texture = texture_hash.and_then(|t| self.texture(t));
        let specular = technique.filter(|t| t.specular).map(|_| Specular {
            color: [
                material.constants[3],
                material.constants[4],
                material.constants[5],
            ],
            power: material.constants[13].max(1.0),
            map: slot(1)
                .filter(|_| technique.is_some_and(|t| !t.emissive))
                .and_then(|t| self.uploaded(t))
                .map(|t| t.handle),
        });
        let normal_map = technique
            .filter(|t| t.normal)
            .and_then(|_| slot(2))
            .and_then(|t| self.uploaded(t))
            .map(|t| NormalMap {
                texture: t.handle,
                alpha_x: t.alpha_x,
            });
        let desc = MaterialDesc {
            texture: texture.map(|t| t.0),
            blend: match texture {
                Some((_, true)) if material.flags & BLENDED != 0 => BlendMode::Translucent,
                Some((_, true)) => BlendMode::Masked { cutoff: 0.5 },
                _ => BlendMode::Opaque,
            },
            double_sided: matches!(texture, Some((_, true))),
            color: if texture.is_some() {
                [1.0; 4]
            } else {
                [0.6, 0.6, 0.6, 1.0]
            },
            vertex_color: technique.is_some_and(|t| t.ambient_occlusion),
            // materialData's diffuse and ambient colours; which constants
            // feed them is read from how they vary, not confirmed.
            direct: if technique.is_some() {
                [
                    material.constants[0],
                    material.constants[1],
                    material.constants[2],
                ]
            } else {
                [1.0; 3]
            },
            ambient: if technique.is_some() {
                [
                    material.constants[6],
                    material.constants[7],
                    material.constants[8],
                ]
            } else {
                [1.0; 3]
            },
            specular,
            normal_map,
            depth_bias,
            ..MaterialDesc::default()
        };
        let id = self.define(backend, &desc)?;
        if material_textures(material)
            .iter()
            .all(|&(t, _)| self.has_texture(t))
        {
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
        if desc.blend == BlendMode::Translucent {
            self.translucent.insert(id);
        }
        Ok(id)
    }

    /// Whether material `id` blends over what is behind it (draw it after
    /// the opaque world, farthest first).
    pub fn is_translucent(&self, id: MaterialId) -> bool {
        self.translucent.contains(&id)
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

/// Material flags bit for surfaces blended over what is behind them, such
/// as vehicle glass (flags 0x0091 where the body has 0x0080). Read from
/// which materials carry it; not confirmed in the game's code.
pub const BLENDED: u16 = 0x0001;

/// A material record's base-colour texture (slot 0).
pub fn base_texture(material: &Material) -> Option<u32> {
    material.textures.first().copied().filter(|&t| t != 0)
}

/// The textures a material record draws with, as (hash, holds data rather
/// than colour): the base colour, and the specular, emissive and normal
/// maps its technique uses.
pub fn material_textures(material: &Material) -> Vec<(u32, bool)> {
    let slot = |i: usize| material.textures.get(i).copied().filter(|&t| t != 0);
    let mut out: Vec<(u32, bool)> = slot(0).map(|t| (t, false)).into_iter().collect();
    if let Some(t) = Technique::from_hash(material.technique) {
        if t.emissive {
            out.extend(slot(1).map(|h| (h, false)));
        } else if t.specular {
            out.extend(slot(1).map(|h| (h, true)));
        }
        if t.normal {
            out.extend(slot(2).map(|h| (h, true)));
        }
    }
    out
}

/// Whether a normal map keeps x in alpha (the DXT5 swizzle, red left
/// constant) rather than in red: whichever of the two varies more.
fn alpha_holds_x(t: &DecodedTexture) -> bool {
    // A small mip is enough to tell.
    let Some(m) = t
        .mips
        .iter()
        .find(|m| m.len() <= 64 * 64 * 4)
        .or(t.mips.last())
    else {
        return false;
    };
    let spread = |c: usize| {
        let n = (m.len() / 4).max(1) as f64;
        let mean = m.chunks_exact(4).map(|p| f64::from(p[c])).sum::<f64>() / n;
        m.chunks_exact(4)
            .map(|p| (f64::from(p[c]) - mean).powi(2))
            .sum::<f64>()
            / n
    };
    spread(3) > spread(0)
}

/// Uploads a decoded texture, generating any mips its chain lacks.
pub fn upload_texture(
    backend: &mut dyn RenderBackend,
    t: &DecodedTexture,
) -> Result<(TextureHandle, bool), BackendError> {
    let desc = TextureDesc {
        width: t.width,
        height: t.height,
        format: if t.linear {
            TextureFormat::Rgba8Unorm
        } else {
            TextureFormat::Rgba8Srgb
        },
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
    /// The node its segment places it at (skinned or not), for showing
    /// and hiding parts.
    pub at: u16,
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
    /// builds it, [`build`](Self::build) does not. A merged batch has none
    /// of its own: its buildings' are in `members`.
    pub collision: Option<CollisionShape>,
    /// The collision of a merged batch's buildings, one each.
    pub members: Vec<MemberShape>,
    /// Per node, whether its parts are hidden: intact, and wrecked (see
    /// [`PartStates`](m2_assets::model::PartStates)). Empty when the model
    /// switches nothing.
    pub hidden: [Vec<bool>; 2],
}

/// The collision of one building of a merged batch (a `TINY` model).
#[derive(Clone, Debug)]
pub struct MemberShape {
    pub shape: CollisionShape,
    /// Where the shape goes: the building's own place in the world, when
    /// it is its own model's `PHY2`; `None` for a shape cut from the
    /// batch's geometry, which goes where the batch does.
    pub placed: Option<(Vec3, Quat)>,
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
        let hidden = model.states.as_ref().map_or_else(Default::default, |s| {
            [s.hidden(&model.nodes, false), s.hidden(&model.nodes, true)]
        });
        for (segment, geometry) in model.lod(lod) {
            let shown = !hidden[0]
                .get(usize::from(segment.node))
                .copied()
                .unwrap_or(false);
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
                    if shown {
                        lo = lo.min(plo);
                        hi = hi.max(phi);
                    }
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
                    at: segment.node,
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
            members: Vec::new(),
            hidden,
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

    /// Textures the model's materials use, as [`material_textures`] lists
    /// them.
    pub fn texture_requests(&self) -> Vec<(u32, bool)> {
        let mut out: Vec<(u32, bool)> = self.materials.iter().flat_map(material_textures).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// One primitive group on the GPU.
#[derive(Debug)]
struct Piece {
    vertex_buffer: VertexBufferHandle,
    index_buffer: IndexBufferHandle,
    local: Mat4,
    /// First index, index count, material, and whether it blends.
    ranges: Vec<(u32, u32, MaterialId, bool)>,
    centre: Vec3,
    bones: Vec<u16>,
    node: Option<u16>,
    at: u16,
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
    /// Per node, hidden intact and wrecked ([`CpuModel::hidden`]).
    hidden: [Vec<bool>; 2],
    /// Draw the wrecked parts instead of the intact ones.
    pub wrecked: bool,
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
                        let id = ids.get(m).copied().unwrap_or_default();
                        (first, count, id, materials.is_translucent(id))
                    })
                    .collect(),
                // Kept in geometry space, so any transform places it.
                centre: p.local.inverse().transform_point3(p.centre),
                bones: p.bones.clone(),
                node: p.node,
                at: p.at,
            });
        }
        Ok(Self {
            hash: cpu.hash,
            pieces,
            bounds: cpu.bounds,
            skipped_groups: cpu.skipped_groups,
            hidden: cpu.hidden.clone(),
            wrecked: false,
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
        materials.load_textures(backend, source, cpu.texture_requests())?;
        Ok(Self::upload(backend, materials, &cpu)?)
    }

    /// Queues every piece at `world`, skinned geometry in its bind pose.
    pub fn draw(&self, queue: &mut DrawQueue, world: Mat4) {
        self.draw_as(queue, world, self.wrecked);
    }

    /// As [`draw`](Self::draw), intact or `wrecked` whatever
    /// [`wrecked`](Self::wrecked) says (one model drawn in many places).
    pub fn draw_as(&self, queue: &mut DrawQueue, world: Mat4, wrecked: bool) {
        for piece in self.shown(wrecked) {
            Self::push(queue, piece, world, None);
        }
    }

    /// Whether the model has wrecked parts to show.
    pub fn has_wreck(&self) -> bool {
        self.hidden[0] != self.hidden[1]
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
        for piece in self.shown(self.wrecked) {
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

    /// The pieces drawn as the model stands, intact or wrecked.
    fn shown(&self, wrecked: bool) -> impl Iterator<Item = &Piece> {
        let hidden = &self.hidden[usize::from(wrecked)];
        self.pieces
            .iter()
            .filter(|p| !hidden.get(usize::from(p.at)).copied().unwrap_or(false))
    }

    fn push(queue: &mut DrawQueue, piece: &Piece, world: Mat4, skin: Option<SkinHandle>) {
        Self::push_at(queue, piece, world * piece.local, skin);
    }

    /// Queues `piece` with geometry-to-world `transform`.
    fn push_at(queue: &mut DrawQueue, piece: &Piece, transform: Mat4, skin: Option<SkinHandle>) {
        let origin = transform.transform_point3(piece.centre);
        for &(first, count, material, translucent) in &piece.ranges {
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
                translucent,
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
            linear: false,
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
            linear: false,
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
        assert!(!cache.is_translucent(m));
        // Glass: the blend bit set.
        let glass = Material {
            flags: 0x0091,
            ..material(4, 11)
        };
        let m = cache.material(&mut backend, &glass).unwrap();
        assert_eq!(backend.material(m).blend, BlendMode::Translucent);
        assert!(cache.is_translucent(m));

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
