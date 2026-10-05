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
use std::sync::OnceLock;

use m2_assets::model::{GeometryKind, Material, ModelInfo, TOPOLOGY};
use m2_assets::vertex::DeclUsage;
use m2_assets::{Model, Ucfx};
use m2_math::glam::{Mat4, Quat, Vec3};
use m2_render::{
    BackendError, BlendMode, DrawCall, DrawQueue, Emissive, Geometry, IndexBufferDesc,
    IndexBufferHandle, Layer, MaterialDesc, MaterialId, NormalMap, Primitive, Reflection,
    RenderBackend, SkinHandle, Specular, TextureDesc, TextureFormat, TextureHandle, Topology,
    VertexBufferDesc, VertexBufferHandle, generate_mips,
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

/// What a model material is cached by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MaterialKey {
    name: u32,
    textures: Vec<u32>,
    technique: u32,
    flags: u16,
    constants: [u32; m2_assets::model::MATERIAL_CONSTANTS],
    depth_bias: bool,
}

/// Textures and materials shared between models.
#[derive(Debug)]
pub struct MaterialCache {
    /// `None`: the texture could not be read; its materials draw untextured.
    textures: HashMap<u32, Option<Uploaded>>,
    /// By everything in the record that shapes the material, and depth
    /// bias: records sharing a name can differ in flags, constants and
    /// maps (the AMX-30's tracks are two-sided, the M1A1's of the same
    /// name not).
    materials: HashMap<MaterialKey, MaterialId>,
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
            max_texture_width: MODEL_TEXTURE_WIDTH,
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

    /// Adds `cpu`'s textures: those it decoded ahead as they are, the rest
    /// read from `source`.
    pub fn load_model_textures<R: Read + Seek>(
        &mut self,
        backend: &mut dyn RenderBackend,
        source: &mut AssetSource<R>,
        cpu: &CpuModel,
    ) -> Result<(), BackendError> {
        for (hash, t) in &cpu.textures {
            if !self.has_texture(*hash) {
                self.add_texture(backend, *hash, t.as_ref())?;
            }
        }
        self.load_textures(backend, source, cpu.texture_requests())
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
        let key = MaterialKey {
            name: material.name_hash,
            textures: material.textures.clone(),
            technique: material.technique,
            flags: material.flags,
            constants: material.constants.map(f32::to_bits),
            depth_bias,
        };
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
        // Emis techniques: slot 1 is the emissive map, constants 9-11 its
        // colour (lamps 2 to 4, everything else 0).
        let emissive = technique.filter(|t| t.emissive).and_then(|_| {
            let color = [
                material.constants[9],
                material.constants[10],
                material.constants[11],
            ];
            (color.iter().any(|&c| c > 0.0)).then(|| Emissive {
                color,
                map: slot(1).and_then(|t| self.uploaded(t)).map(|t| t.handle),
            })
        });
        // Refl and Metal techniques mirror the surroundings, scaled by
        // constant 14 (the shaders' constant0.x) and the specular map.
        // Metal also weights it by Schlick's Fresnel. Its reflectance facing
        // the eye is taken from constant 15 read as an index of refraction
        // (only Metal materials set it, 1 to 3.34, mostly 2.705: about 0.21
        // facing); that reading is inferred, not confirmed.
        let reflection = technique
            .filter(|t| t.reflection || t.metal)
            .map(|_| material.constants[14])
            .filter(|&s| s > 0.0)
            .map(|strength| Reflection {
                strength,
                fresnel: technique.filter(|t| t.metal).map(|_| {
                    let n = material.constants[15].max(1.0);
                    ((n - 1.0) / (n + 1.0)).powi(2)
                }),
            });
        let normal_map = technique
            .filter(|t| t.normal)
            .and_then(|_| slot(2))
            .and_then(|t| self.uploaded(t))
            .map(|t| NormalMap {
                texture: t.handle,
                alpha_x: t.alpha_x,
            });
        let has = |bit: u16| material.flags & bit != 0;
        let alpha = matches!(texture, Some((_, true)));
        let blend = if has(flags::ADDITIVE) {
            BlendMode::Additive
        } else if has(flags::BLENDED) {
            BlendMode::Translucent
        } else if alpha || has(flags::CUT_OUT) {
            BlendMode::Masked { cutoff: 0.5 }
        } else {
            BlendMode::Opaque
        };
        // The opacity constant scales the texture's alpha.
        let opacity = material.constants[12].clamp(0.0, 1.0);
        let desc = MaterialDesc {
            texture: texture.map(|t| t.0),
            blend,
            double_sided: has(flags::TWO_SIDED) || alpha,
            color: if texture.is_some() {
                [1.0, 1.0, 1.0, opacity]
            } else {
                [0.6, 0.6, 0.6, opacity]
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
            emissive,
            reflection,
            rim: technique.is_some_and(|t| t.rim),
            depth_bias,
            // A beam of light (a searchlight's cone): blended, glowing
            // with its own texture. It fades where its sides turn away
            // from the eye, so it reads as a soft shaft rather than a
            // solid cone, as in footage of the game (the game's own
            // falloff is not known).
            soft_edge: has(flags::BLENDED) && emissive.is_some() && slot(1) == slot(0),
            ..MaterialDesc::default()
        };
        let tinted = debug_tint(material, desc.texture.is_some());
        let desc = if tinted {
            MaterialDesc {
                texture: None,
                color: [1.0, 0.0, 1.0, opacity],
                ..desc
            }
        } else {
            desc
        };
        let id = self.define(backend, &desc)?;
        if material_textures(material)
            .iter()
            .all(|&(t, _)| self.has_texture(t))
        {
            if tinted {
                eprintln!(
                    "tinted material {:08x}: technique {:08x}, flags {:04x}, textures {:08x?}{}",
                    material.name_hash,
                    material.technique,
                    material.flags,
                    material.textures,
                    if depth_bias {
                        " (terrain cell mesh)"
                    } else {
                        ""
                    }
                );
            }
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
        if desc.blend.is_translucent() {
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

/// Widest model texture read by default; bigger mips are skipped.
pub const MODEL_TEXTURE_WIDTH: u32 = 2048;

/// Material flags bits, read from which materials carry them (a survey of
/// every model material); not confirmed in the game's code.
pub mod flags {
    /// Alpha blended over what is behind: glass, blob shadows, water,
    /// boards (vehicle glass is 0x0091 where the body is 0x0080).
    pub const BLENDED: u16 = 0x0001;
    /// Added to what is behind: lights, neon, light cones, trails.
    pub const ADDITIVE: u16 = 0x0002;
    /// Cut out by the texture's alpha: leaves, hedges, hair, nets.
    pub const CUT_OUT: u16 = 0x0008;
    /// Both faces drawn: hair, wires, plants, helmets' visors.
    pub const TWO_SIDED: u16 = 0x0010;
    /// Never drawn: every material carrying it is coloured with the
    /// default normal map.
    pub const HIDDEN: u16 = 0x0800;
}

/// Whether to draw `material` solid magenta, to find a surface on screen:
/// `M2_TINT_FLAGS=<hex mask>` picks materials with any of those flag bits,
/// `M2_TINT_MATERIAL=<hex name>,...` those named, `M2_TINT_UNTEXTURED=1`
/// those drawn without a base texture. Each one is logged once its
/// textures are in.
fn debug_tint(material: &Material, textured: bool) -> bool {
    struct Settings {
        mask: u16,
        names: HashSet<u32>,
        untextured: bool,
    }
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    let hex = |v: &str| u32::from_str_radix(v.trim().trim_start_matches("0x"), 16).ok();
    let s = SETTINGS.get_or_init(|| Settings {
        mask: std::env::var("M2_TINT_FLAGS")
            .ok()
            .and_then(|v| hex(&v))
            .map_or(0, |m| m as u16),
        names: std::env::var("M2_TINT_MATERIAL")
            .map(|v| v.split(',').filter_map(hex).collect())
            .unwrap_or_default(),
        untextured: std::env::var_os("M2_TINT_UNTEXTURED").is_some(),
    });
    material.flags & s.mask != 0
        || s.names.contains(&material.name_hash)
        || (s.untextured && !textured)
}

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
    /// How `indices` are drawn: the model's strips, or a triangle list
    /// where ranges were split by member.
    pub topology: Topology,
    /// For a merged batch (`TINY` geometry), the member each range draws
    /// (by position in the batch's [`tiny_members`](CpuModel::tiny_members)),
    /// so a member drawn by its own model can be left out; else empty.
    pub members: Vec<u16>,
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
    /// Textures decoded ahead (on a loading thread, by
    /// [`decode_textures`](Self::decode_textures)), for
    /// [`MaterialCache::load_model_textures`] to upload without reading.
    pub textures: Vec<(u32, Option<DecodedTexture>)>,
    /// For a merged batch, the entity each member is (see
    /// [`Model::tiny_members`]); empty otherwise.
    pub tiny_members: Vec<u32>,
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
                let ranges: Vec<(u32, u32, usize)> = group
                    .material_ranges
                    .iter()
                    .filter(|r| r.start_index + r.index_count() <= group.indices.len() as u32)
                    .filter(|r| {
                        model
                            .materials
                            .get(r.material as usize)
                            .is_none_or(|m| m.flags & flags::HIDDEN == 0)
                    })
                    .map(|r| (r.start_index, r.index_count(), r.material as usize))
                    .collect();
                let (ranges, indices, topology, members) = if geometry.kind == GeometryKind::Tiny {
                    let (r, i, m) = split_members(group, &ranges);
                    (r, i, Topology::TriangleList, m)
                } else {
                    (ranges, group.index_bytes(), TOPOLOGY, Vec::new())
                };
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
                    index_desc: IndexBufferDesc {
                        index_count: (indices.len() / 2) as u32,
                        ..group.index_buffer_desc()
                    },
                    indices,
                    local,
                    ranges,
                    topology,
                    members,
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
            textures: Vec::new(),
            tiny_members: if model.info.flags & ModelInfo::FLAG_TINY != 0 {
                model.tiny_members()
            } else {
                Vec::new()
            },
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

    /// Reads and decodes the model's textures (up to `max_width` wide) into
    /// [`textures`](Self::textures), so a loading thread does the work
    /// rather than the frame that uploads the model.
    pub fn decode_textures<R: Read + Seek>(&mut self, source: &mut AssetSource<R>, max_width: u32) {
        self.textures = self
            .texture_requests()
            .into_iter()
            .map(|(hash, linear)| {
                let t = source.texture(hash, max_width).ok().map(|mut t| {
                    t.linear = linear;
                    t
                });
                (hash, t)
            })
            .collect();
    }
}

/// (first index, index count, material index) draws, their index bytes
/// and each draw's member.
type SplitRanges = (Vec<(u32, u32, usize)>, Vec<u8>, Vec<u16>);

/// Splits a merged batch's strip `ranges` by member (each vertex's
/// position `w` is the member it belongs to), as triangle lists: the new
/// ranges, their indices as bytes, and each range's member.
fn split_members(
    group: &m2_assets::model::PrimitiveGroup<'_>,
    ranges: &[(u32, u32, usize)],
) -> SplitRanges {
    let member_of: Vec<u16> = group
        .decl
        .find(DeclUsage::Position, 0)
        .and_then(|e| group.vertices.read(e))
        .unwrap_or_default()
        .iter()
        .map(|p| p[3].max(0.0).round() as u16)
        .collect();
    let (mut out, mut indices, mut members) = (Vec::new(), Vec::new(), Vec::new());
    for &(first, count, material) in ranges {
        let mut by_member: std::collections::BTreeMap<u16, Vec<u16>> = Default::default();
        for tri in m2_assets::model::strip_triangles(&group.indices, first, count) {
            let member = member_of.get(usize::from(tri[0])).copied().unwrap_or(0);
            by_member.entry(member).or_default().extend(tri);
        }
        for (member, tris) in by_member {
            out.push(((indices.len() / 2) as u32, tris.len() as u32, material));
            indices.extend(tris.iter().flat_map(|i| i.to_le_bytes()));
            members.push(member);
        }
    }
    (out, indices, members)
}

/// One primitive group on the GPU.
#[derive(Debug)]
struct Piece {
    vertex_buffer: VertexBufferHandle,
    index_buffer: IndexBufferHandle,
    local: Mat4,
    /// First index, index count, material, and whether it blends.
    ranges: Vec<(u32, u32, MaterialId, bool)>,
    topology: Topology,
    /// Per range, its batch member (empty: not a batch).
    members: Vec<u16>,
    /// Per range, whether its material could be a vehicle's track: cut
    /// out, as tracks' are (the AMX-30's are two-sided as well, the
    /// M1A1's not). A model's coarser levels of detail aren't uploaded, so
    /// a cut-out LOD material isn't drawn with them.
    track: Vec<bool>,
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
    /// For a merged batch, the entity each member is.
    pub tiny_members: Vec<u32>,
    /// A tracked vehicle: its track materials scroll by
    /// [`track_travel`](Self::track_travel).
    pub tracks: bool,
    /// Metres the tracks have run.
    pub track_travel: f32,
}

/// How far a track's texture moves along v per metre the track runs.
/// Invented: the game scrolls its tracks from code.
const TRACK_V_PER_METRE: f32 = 0.5;

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
        // Which model draws a tinted material, as its textures may never
        // all load to log it by.
        let mut tinted: Vec<usize> = cpu
            .pieces
            .iter()
            .flat_map(|p| p.ranges.iter().map(|r| r.2))
            .filter(|&m| cpu.materials.get(m).is_some_and(|m| debug_tint(m, true)))
            .collect();
        tinted.sort_unstable();
        tinted.dedup();
        for m in tinted {
            let m = &cpu.materials[m];
            eprintln!(
                "model {:08x} draws tinted material {:08x} (flags {:04x}, textures {:08x?})",
                cpu.hash, m.name_hash, m.flags, m.textures
            );
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
                topology: p.topology,
                members: p.members.clone(),
                track: p
                    .ranges
                    .iter()
                    .map(|&(_, _, m)| {
                        cpu.materials
                            .get(m)
                            .is_some_and(|m| m.flags & flags::CUT_OUT != 0)
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
            tiny_members: cpu.tiny_members.clone(),
            tracks: false,
            track_travel: 0.0,
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
            self.push(queue, piece, world, None);
        }
    }

    /// Queues a merged batch at `world` without the members `hide` picks
    /// (by position in [`tiny_members`](Self::tiny_members)): those drawn
    /// by their own models.
    pub fn draw_batch(&self, queue: &mut DrawQueue, world: Mat4, hide: &dyn Fn(u16) -> bool) {
        for piece in self.shown(false) {
            self.push_ranges(queue, piece, world * piece.local, None, |k| {
                piece.members.get(k).is_none_or(|&m| !hide(m))
            });
        }
    }

    /// As [`draw_as`](Self::draw_as), with each rigid piece at its node's
    /// place in the model as `nodes` has it (a loose prop's pieces flying,
    /// [`Loose::posed`](crate::Loose::posed)); skinned pieces stay put.
    /// Pieces on nodes `flying` marks are drawn whatever the state shows
    /// (a broken prop's, [`Loose::flying`](crate::Loose::flying)).
    pub fn draw_moved(
        &self,
        queue: &mut DrawQueue,
        world: Mat4,
        nodes: &[Mat4],
        wrecked: bool,
        flying: &[bool],
    ) {
        let hidden = &self.hidden[usize::from(wrecked)];
        let at = |p: &Piece, marks: &[bool]| marks.get(usize::from(p.at)).copied().unwrap_or(false);
        for piece in self
            .pieces
            .iter()
            .filter(|p| at(p, flying) || !at(p, hidden))
        {
            match piece.node.and_then(|n| nodes.get(usize::from(n))) {
                Some(&m) if piece.bones.is_empty() => self.push_at(queue, piece, world * m, None),
                _ => self.push(queue, piece, world, None),
            }
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
                    self.push_at(queue, piece, world * m, None);
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
            self.push(queue, piece, world, handle);
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

    fn push(&self, queue: &mut DrawQueue, piece: &Piece, world: Mat4, skin: Option<SkinHandle>) {
        self.push_at(queue, piece, world * piece.local, skin);
    }

    /// Queues `piece` with geometry-to-world `transform`.
    fn push_at(
        &self,
        queue: &mut DrawQueue,
        piece: &Piece,
        transform: Mat4,
        skin: Option<SkinHandle>,
    ) {
        self.push_ranges(queue, piece, transform, skin, |_| true);
    }

    /// Queues the ranges of `piece` that `keep` picks (by index).
    fn push_ranges(
        &self,
        queue: &mut DrawQueue,
        piece: &Piece,
        transform: Mat4,
        skin: Option<SkinHandle>,
        keep: impl Fn(usize) -> bool,
    ) {
        let origin = transform.transform_point3(piece.centre);
        for (k, &(first, count, material, translucent)) in piece.ranges.iter().enumerate() {
            if !keep(k) {
                continue;
            }
            queue.push(Primitive {
                call: DrawCall {
                    material,
                    topology: piece.topology,
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
                    uv_offset: if self.tracks && piece.track.get(k).copied().unwrap_or(false) {
                        [0.0, (self.track_travel * TRACK_V_PER_METRE).fract()]
                    } else {
                        [0.0; 2]
                    },
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
        // Glass: the blend bit set, its opacity in the colour's alpha.
        let mut glass = Material {
            flags: 0x0091,
            ..material(4, 11)
        };
        glass.constants[12] = 0.5;
        let m = cache.material(&mut backend, &glass).unwrap();
        assert_eq!(backend.material(m).blend, BlendMode::Translucent);
        assert_eq!(backend.material(m).color[3], 0.5);
        assert!(backend.material(m).double_sided);
        assert!(cache.is_translucent(m));
        // A light: added, even without alpha.
        let light = Material {
            flags: 0x0012,
            ..material(5, 10)
        };
        let m = cache.material(&mut backend, &light).unwrap();
        assert_eq!(backend.material(m).blend, BlendMode::Additive);
        assert!(cache.is_translucent(m));
        // A lamp lens: PgDiffEmisFP glows with constants 9-11 through slot 1.
        let mut lamp = Material {
            technique: 0x7ed2_de27,
            textures: vec![10, 10],
            ..material(6, 10)
        };
        lamp.constants[9..12].copy_from_slice(&[4.0, 4.0, 2.5]);
        let m = cache.material(&mut backend, &lamp).unwrap();
        let glow = backend.material(m).emissive.unwrap();
        assert_eq!(glow.color, [4.0, 4.0, 2.5]);
        assert!(glow.map.is_some());
        // Paint: PgDiffSpecMetalNormFP mirrors by constant 14, weighted by
        // Fresnel from constant 15.
        let mut paint = Material {
            technique: 0xb68e_1be7,
            ..material(7, 10)
        };
        paint.constants[14] = 0.25;
        paint.constants[15] = 3.0;
        let m = cache.material(&mut backend, &paint).unwrap();
        let shine = backend.material(m).reflection.unwrap();
        assert_eq!(shine.strength, 0.25);
        assert_eq!(shine.fresnel, Some(0.25));

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
