//! The island's layout: where its terrain cells, low-resolution tiles and
//! placed models are, and their terrain decoded into upload-ready meshes.
//!
//! [`WorldIndex::build`] reads only the entity layers (finding them through
//! the archive's asset table, so the rest of the 2.5 GB archive is not
//! decoded) and records the `TerrainObject`, `LowResTerrainObject` and
//! `ModelName` placements, and the people the layers place (soldiers at
//! their posts and on their patrol routes). Terrain is read later, cell by cell, by
//! [`terrain_cell`] and [`low_res_tile`].
//!
//! # Provenance
//!
//! Format-from-samples, as `m2-assets`; this module only combines what the
//! parsers there decode, following the glTF exporter's world export.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};

use m2_assets::layer::{LAYER_TYPE, TEMPLATE_DB_TYPE};
use m2_assets::model::MODEL_TYPE;
use m2_assets::model::Material;
use m2_assets::path::PATH_TYPE;
use m2_assets::terrain::{CELL_SIZE, TerrainGroup, TerrainMaterial, colour_map_uv};
use m2_assets::vertex::DeclUsage;
use m2_assets::{BlockContents, Layer, LowResTile, Path, TerrainCell, name_hash};
use m2_math::glam::{Quat, Vec3};
use m2_render::{
    BufferUsage, IndexBufferDesc, IndexFormat, VertexBufferDesc, VertexFormat, VertexLayout,
    VertexSemantic,
};

use crate::assets::{AssetError, AssetSource};
use crate::physics::CollisionShape;
use crate::technique::Technique;

/// Side of the island in metres (20 cells of 400 m).
pub const ISLAND: f32 = 8000.0;

/// Skirt depth under full-detail cells and under low-resolution tiles,
/// whose coarser edges leave wider gaps.
const CELL_SKIRT: f32 = 4.0;
const TILE_SKIRT: f32 = 30.0;

/// One placed model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectPlacement {
    pub model: u32,
    pub position: Vec3,
    pub rotation: Quat,
    /// Placed by its entity's name (`global_env_rocksbeach01 0x00123abc`
    /// is the model `global_env_rocksbeach01`) rather than a `ModelName`:
    /// the island's rocks, plants, fences and sandbags, kept within
    /// [`WorldSettings::prop_distance`](crate::WorldSettings::prop_distance).
    pub named: bool,
}

/// An entity of the static layers whose name is a vehicle template
/// (`M151 Softtop (VZ)`, `HMMWV (Softtop)`, `Small Fishing Boat`), or a
/// template the database doesn't know: the game's own parked vehicles.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplatePlacement {
    pub entity: u32,
    /// The template (`M151 Softtop (VZ)`), without the entity's `0x…` id.
    pub name: String,
    pub position: Vec3,
    pub rotation: Quat,
    /// The template's model, from the template database's `ModelName`;
    /// `None` when the database doesn't give it one.
    pub model: Option<u32>,
    /// What drives it, from the database's physics components.
    pub kind: VehicleKind,
    /// The name hash of the layer holding it: `layers_static` is many
    /// layers in one block (`vz_mar_village`, ...), which the game loads
    /// by name.
    pub layer: u32,
}

/// What a vehicle template drives as, by which physics component the
/// template database gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VehicleKind {
    /// `_CarPhysicsV2`: cars, trucks, vans, bikes.
    Car,
    /// `_TankPhysics`: tanks and tracked APCs.
    Tank,
    /// `_BoatPhysics`.
    Boat,
    /// `_HelicopterPhysics`.
    Helicopter,
    /// No physics the database names: a template it doesn't define.
    Unknown,
}

/// What the template database says of one template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemplateInfo {
    /// Its `ModelName`, when that is a model the archive holds.
    pub model: Option<u32>,
    /// Its physics component's kind, when it has one.
    pub vehicle: Option<VehicleKind>,
}

/// The template database by template name, lower case.
#[derive(Clone, Debug, Default)]
pub struct Templates(pub HashMap<String, TemplateInfo>);

impl Templates {
    /// The template an entity or template name (`M151 Softtop (VZ)
    /// 0x0012abcd`, `AMX30`) names, whatever its case.
    pub fn get(&self, name: &str) -> Option<&TemplateInfo> {
        self.0.get(&member_stem(name).to_ascii_lowercase())
    }
}

impl Templates {
    /// Reads the one template database asset, if the archive has it.
    fn read<R: Read + Seek>(source: &mut AssetSource<R>, models: &HashSet<u32>) -> Self {
        let index = source.archive().index();
        let Some(hash) = index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
        else {
            return Self::default();
        };
        let Ok(data) = source.asset(hash, TEMPLATE_DB_TYPE) else {
            return Self::default();
        };
        let Ok(db) = Layer::parse(&data) else {
            return Self::default();
        };
        let model_of = db.hashes("ModelName");
        let mut kinds: HashMap<u32, VehicleKind> = HashMap::new();
        for (component, kind) in [
            ("_CarPhysicsV2", VehicleKind::Car),
            ("_TankPhysics", VehicleKind::Tank),
            ("_BoatPhysics", VehicleKind::Boat),
            ("_HelicopterPhysics", VehicleKind::Helicopter),
        ] {
            let Some(c) = db.component(component) else {
                continue;
            };
            for (id, _) in c.records().unwrap_or_default() {
                kinds.insert(id, kind);
            }
        }
        Self(
            db.names()
                .into_iter()
                .map(|(id, name)| {
                    (
                        name.to_ascii_lowercase(),
                        TemplateInfo {
                            model: model_of.get(&id).copied().filter(|m| models.contains(m)),
                            vehicle: kinds.get(&id).copied(),
                        },
                    )
                })
                .collect(),
        )
    }
}

/// Components of people and vehicles, which a state layer's models
/// leave to the game.
const NOT_PROPS: [&str; 5] = ["Ai", "AiPatrol", "Rider", "_TankPhysics", "PopulationFlow"];

/// The scripts keep what they don't need yet about 40 km under the island:
/// below this height it isn't placed.
const HIDDEN_BELOW: f32 = -1000.0;

/// Components whose entities are never a template placement: roads,
/// paths, regions, lights, sounds and such.
const NOT_TEMPLATES: [&str; 16] = [
    "Road",
    "RoadIntersection",
    "Path",
    "LineRegion",
    "PointLocation",
    "LightObject",
    "ScrubObject",
    "TerrainObject",
    "LowResTerrainObject",
    "PopulationDensity",
    "PopulationSimpleSpawner",
    "MusicRegion",
    "MusicSource",
    "SoundAmbience",
    "AtmosphereBase",
    "LandingZone",
];

/// A person a layer places: a soldier at a post or on a patrol route,
/// from the `PopulationSimpleSpawner`, `Ai` and `AiPatrol` components.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnPoint {
    pub entity: u32,
    /// The entity's name (`VZ Soldier 03`, ...): the only hint of who it
    /// spawns, as which model a spawner uses is not decoded.
    pub name: String,
    pub position: Vec3,
    pub rotation: Quat,
    /// The patrol route its `AiPatrol` names, world space; empty for a
    /// post.
    pub route: Vec<Vec3>,
}

/// Where everything on the island is.
#[derive(Clone, Debug, Default)]
pub struct WorldIndex {
    /// Terrain cell asset hash to cell centre.
    pub cells: HashMap<u32, Vec3>,
    /// Low-resolution tile asset hash to tile centre.
    pub tiles: HashMap<u32, Vec3>,
    /// Models placed by the static layers (the world as it always is,
    /// without mission states).
    pub objects: Vec<ObjectPlacement>,
    /// People the layers place, all layers.
    pub spawns: Vec<SpawnPoint>,
    /// Every placed entity of the static layers by id: the buildings of
    /// the merged town batches are among them (see
    /// [`Model::tiny_members`](m2_assets::Model::tiny_members)).
    pub members: HashMap<u32, Member>,
    /// Entities of the static layers placed by a template name that is a
    /// vehicle, or that the template database doesn't define.
    pub templates: Vec<TemplatePlacement>,
    /// The template database, for names the scripts and mission layers
    /// use.
    pub template_db: Templates,
    /// Models the state layers place (blocks of their own, which the
    /// mission scripts load and unload: `vz_state_vzacon001`), by entity:
    /// what stands while the layer is in, such as a mission's gates.
    /// Vehicles and people are left out (the alpha places those itself).
    pub state_objects: HashMap<u32, ObjectPlacement>,
}

/// A placed entity of the static layers, as a building of a merged batch.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub position: Vec3,
    pub rotation: Quat,
    /// Model hashes that may be its own model, likeliest first: its
    /// `ModelName`, if it has one, then the hash of its name's stem
    /// (`_commercial_bld_corner8x16a 26` is `commercial_bld_corner8x16a`).
    pub models: Vec<u32>,
}

/// The layer a block holds, from its path:
/// `blocks\VZ\vz_state_vzacon001_P000_Q3.block` is `vz_state_vzacon001`.
/// (`layers_static` is many layers in one block, keyed by name hash.)
pub fn layer_name(block_path: &str) -> String {
    let file = block_path.rsplit(['\\', '/']).next().unwrap_or(block_path);
    let stem = file.strip_suffix(".block").unwrap_or(file);
    // Drop a trailing `_P<digits>_Q<digits>` (the block's partition).
    let mut parts: Vec<&str> = stem.split('_').collect();
    let tagged = |p: &str, c: char| {
        p.len() > 1 && p.starts_with(c) && p[1..].bytes().all(|b| b.is_ascii_digit())
    };
    if parts.len() > 2 && tagged(parts[parts.len() - 1], 'Q') && tagged(parts[parts.len() - 2], 'P')
    {
        parts.truncate(parts.len() - 2);
    }
    parts.join("_").to_ascii_lowercase()
}

/// An entity name without its leading `_` and its trailing ` 26` or
/// ` 0x001384eb`: the name of the model or template it is. A name with
/// no such id (`AMX30 (Full)`) is kept whole.
pub fn member_stem(name: &str) -> &str {
    let name = name.trim_start_matches('_');
    match name.rsplit_once(' ') {
        Some((stem, id)) if is_id(id) => stem,
        _ => name,
    }
}

/// `26` or `0x001384eb`.
fn is_id(token: &str) -> bool {
    let digits = |t: &str, radix| !t.is_empty() && t.chars().all(|c| c.is_digit(radix));
    token
        .strip_prefix("0x")
        .map_or(digits(token, 10), |hex| digits(hex, 16))
}

/// A designer's test entity (`68valiant test`, `RIB36 Test`).
fn is_test(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(" test")
}

/// Which names the archive holds, for placing entities by name.
struct Assets {
    /// Name hashes of the models.
    models: HashSet<u32>,
    /// Name hashes of every asset of every type.
    any: HashSet<u32>,
    /// The template database.
    templates: Templates,
}

/// What [`WorldIndex::build`] gathers to find patrol routes: soldier to
/// path entity (`AiPatrol`'s third field), path entity to path asset
/// (`Path`'s field), and every entity's placement.
#[derive(Default)]
struct Routes {
    patrol_of: HashMap<u32, u32>,
    path_of: HashMap<u32, u32>,
    placed: HashMap<u32, (Vec3, Quat)>,
}

fn rotation(q: [f32; 4]) -> Quat {
    let r = Quat::from_array(q);
    if r.is_finite() && r.length() > 0.5 {
        r.normalize()
    } else {
        Quat::IDENTITY
    }
}

impl WorldIndex {
    /// Reads every entity layer in the archive.
    pub fn build<R: Read + Seek>(source: &mut AssetSource<R>) -> Result<Self, AssetError> {
        let index = source.archive().index();
        let layer_blocks: HashSet<usize> = index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == LAYER_TYPE)
            .map(|a| usize::from(a.block))
            .collect();
        let mut blocks: Vec<(usize, bool)> = layer_blocks
            .into_iter()
            .map(|b| (b, index.blocks()[b].path.contains("layers_static")))
            .collect();
        blocks.sort_unstable();
        let mut assets = Assets {
            models: index
                .assets()
                .iter()
                .filter(|a| index.type_hash(a) == MODEL_TYPE)
                .map(|a| a.name_hash)
                .collect(),
            any: index.assets().iter().map(|a| a.name_hash).collect(),
            templates: Templates::default(),
        };
        assets.templates = Templates::read(source, &assets.models);

        let mut world = Self::default();
        let mut routes = Routes::default();
        for (block, is_static) in blocks {
            let decoded = source.block(block)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash != LAYER_TYPE {
                    continue;
                }
                let Ok(layer) = Layer::parse(data) else {
                    continue;
                };
                world.add_layer(&layer, e.name_hash, is_static, &assets);
                world.add_people(&layer, &mut routes);
            }
        }
        world.template_db = assets.templates;
        let mut paths: HashMap<u32, Vec<Vec3>> = HashMap::new();
        for spawn in &mut world.spawns {
            let Some(&target) = routes.patrol_of.get(&spawn.entity) else {
                continue;
            };
            let (Some(&(at, q)), Some(&asset)) =
                (routes.placed.get(&target), routes.path_of.get(&target))
            else {
                continue;
            };
            let points = paths.entry(asset).or_insert_with(|| {
                source
                    .asset(asset, PATH_TYPE)
                    .ok()
                    .and_then(|data| Path::parse(&data).ok())
                    .map(|p| p.points.into_iter().map(Vec3::from).collect())
                    .unwrap_or_default()
            });
            spawn.route = points.iter().map(|&p| at + q * p).collect();
        }
        Ok(world)
    }

    fn add_people(&mut self, layer: &Layer<'_>, routes: &mut Routes) {
        let placements = layer.placements();
        routes.placed.extend(
            placements
                .iter()
                .map(|(&e, p)| (e, (Vec3::from(p.position), rotation(p.rotation)))),
        );
        routes.path_of.extend(layer.hashes("Path"));
        if let Some(c) = layer.component("AiPatrol") {
            for (id, rec) in c.records().unwrap_or_default() {
                if let Some(b) = rec.get(8..12) {
                    routes
                        .patrol_of
                        .insert(id, u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                }
            }
        }
        let names = layer.names();
        let mut seen: HashSet<u32> = HashSet::new();
        for component in ["PopulationSimpleSpawner", "Ai", "AiPatrol"] {
            let Some(c) = layer.component(component) else {
                continue;
            };
            for (id, _) in c.records().unwrap_or_default() {
                let Some(p) = placements.get(&id) else {
                    continue;
                };
                if !seen.insert(id) {
                    continue;
                }
                self.spawns.push(SpawnPoint {
                    entity: id,
                    name: names.get(&id).copied().unwrap_or("").to_owned(),
                    position: Vec3::from(p.position),
                    rotation: rotation(p.rotation),
                    route: Vec::new(),
                });
            }
        }
    }

    fn add_layer(&mut self, layer: &Layer<'_>, layer_hash: u32, is_static: bool, assets: &Assets) {
        let placements = layer.placements();
        let at = |entity| placements.get(&entity);
        for (entity, asset) in layer.hashes("TerrainObject") {
            if let Some(p) = at(entity) {
                self.cells.insert(asset, Vec3::from(p.position));
            }
        }
        for (entity, asset) in layer.hashes("LowResTerrainObject") {
            if let Some(p) = at(entity) {
                self.tiles.insert(asset, Vec3::from(p.position));
            }
        }
        if !is_static {
            self.add_state_objects(layer, assets);
        } else {
            let models = layer.hashes("ModelName");
            for (&entity, model) in &models {
                if let Some(p) = at(entity) {
                    self.objects.push(ObjectPlacement {
                        model: *model,
                        position: Vec3::from(p.position),
                        rotation: rotation(p.rotation),
                        named: false,
                    });
                }
            }
            let not_templates: HashSet<u32> = NOT_TEMPLATES
                .iter()
                .filter_map(|c| layer.component(c))
                .flat_map(|c| c.records().unwrap_or_default())
                .map(|(entity, _)| entity)
                .collect();
            for (entity, name) in layer.names() {
                let Some(p) = at(entity) else { continue };
                let (position, rotation) = (Vec3::from(p.position), rotation(p.rotation));
                let mut candidates: Vec<u32> = models.get(&entity).copied().into_iter().collect();
                let stem = member_stem(name);
                let stem_hash = (!stem.is_empty()).then(|| name_hash(stem));
                candidates.extend(stem_hash);
                candidates.dedup();
                self.members.insert(
                    entity,
                    Member {
                        position,
                        rotation,
                        models: candidates,
                    },
                );
                if models.contains_key(&entity) || not_templates.contains(&entity) {
                    continue;
                }
                let template = assets.templates.get(stem);
                match (stem_hash, template) {
                    // Its name is a model.
                    (Some(h), _) if assets.models.contains(&h) => {
                        self.objects.push(ObjectPlacement {
                            model: h,
                            position,
                            rotation,
                            named: true,
                        })
                    }
                    // A vehicle template that drives: the alpha places it.
                    (
                        _,
                        Some(&TemplateInfo {
                            model,
                            vehicle: Some(kind @ (VehicleKind::Car | VehicleKind::Tank)),
                        }),
                    ) => self.templates.push(TemplatePlacement {
                        entity,
                        name: stem.to_owned(),
                        position,
                        rotation,
                        model,
                        kind,
                        layer: layer_hash,
                    }),
                    // A template with a model that doesn't drive yet (a
                    // boat, a helicopter) or has no physics (a pickup, an
                    // emplacement, a prop): placed as it stands.
                    (
                        _,
                        Some(&TemplateInfo {
                            model: Some(model), ..
                        }),
                    ) => self.objects.push(ObjectPlacement {
                        model,
                        position,
                        rotation,
                        named: true,
                    }),
                    // A name no asset or template defines: the words may
                    // (not the designers' test entities).
                    (Some(h), None) if !assets.any.contains(&h) && !is_test(stem) => {
                        self.templates.push(TemplatePlacement {
                            entity,
                            name: stem.to_owned(),
                            position,
                            rotation,
                            model: None,
                            kind: VehicleKind::Unknown,
                            layer: layer_hash,
                        })
                    }
                    _ => {}
                }
            }
        }
    }

    /// A state layer's models: by `ModelName`, by a name that is a model,
    /// or by a template with a model and no physics (not a vehicle).
    /// People, vehicles and what the scripts keep far under the island
    /// are left out.
    fn add_state_objects(&mut self, layer: &Layer<'_>, assets: &Assets) {
        let placements = layer.placements();
        let models = layer.hashes("ModelName");
        let names = layer.names();
        let not_props: HashSet<u32> = NOT_TEMPLATES
            .iter()
            .chain(&NOT_PROPS)
            .filter_map(|c| layer.component(c))
            .flat_map(|c| c.records().unwrap_or_default())
            .map(|(entity, _)| entity)
            .collect();
        for (&entity, p) in &placements {
            if not_props.contains(&entity) || p.position[1] < HIDDEN_BELOW {
                continue;
            }
            let stem = names.get(&entity).map_or("", |n| member_stem(n));
            let model = models.get(&entity).copied().or_else(|| {
                let h = (!stem.is_empty()).then(|| name_hash(stem))?;
                if assets.models.contains(&h) {
                    return Some(h);
                }
                match assets.templates.get(stem) {
                    Some(&TemplateInfo {
                        model: Some(m),
                        vehicle: None,
                    }) => Some(m),
                    _ => None,
                }
            });
            if let Some(model) = model {
                self.state_objects.insert(
                    entity,
                    ObjectPlacement {
                        model,
                        position: Vec3::from(p.position),
                        rotation: rotation(p.rotation),
                        named: !models.contains_key(&entity),
                    },
                );
            }
        }
    }

    /// Cells whose square comes within `radius` of `(x, z)`, nearest first.
    pub fn cells_near(&self, x: f32, z: f32, radius: f32) -> Vec<(u32, Vec3)> {
        let mut out: Vec<(f32, u32, Vec3)> = self
            .cells
            .iter()
            .map(|(&h, &c)| (square_distance(c, x, z), h, c))
            .filter(|&(d, _, _)| d <= radius)
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out.into_iter().map(|(_, h, c)| (h, c)).collect()
    }

    /// A height to start at near `(x, z)`: the centre height of the nearest
    /// cell, or 0.
    pub fn rough_height(&self, x: f32, z: f32) -> f32 {
        self.cells
            .values()
            .min_by(|a, b| square_distance(**a, x, z).total_cmp(&square_distance(**b, x, z)))
            .map_or(0.0, |c| c.y)
    }
}

/// Distance from `(x, z)` to the 400 m square centred on `centre`.
pub fn square_distance(centre: Vec3, x: f32, z: f32) -> f32 {
    let dx = ((centre.x - x).abs() - CELL_SIZE / 2.0).max(0.0);
    let dz = ((centre.z - z).abs() - CELL_SIZE / 2.0).max(0.0);
    (dx * dx + dz * dz).sqrt()
}

/// Bytes per [`TerrainMesh`] vertex.
pub const TERRAIN_STRIDE: usize = 52;
/// Bytes per vertex of [`simple_layout`].
pub const SIMPLE_STRIDE: usize = 32;

/// Position, normal and texture coordinate, as `f32`s: the sea and ground
/// scatter.
pub fn simple_layout() -> VertexLayout {
    VertexLayout::packed(&[
        (VertexSemantic::Position, VertexFormat::Float3),
        (VertexSemantic::Normal, VertexFormat::Float3),
        (VertexSemantic::TexCoord(0), VertexFormat::Float2),
    ])
    .expect("valid layout")
}

/// What one range of a terrain mesh is drawn with.
#[derive(Clone, Debug, PartialEq)]
pub enum TerrainSurface {
    /// Ground: the cell's colour map over the whole cell, the shared detail
    /// texture, and up to four tiled layers weighted by the vertex colour.
    Ground {
        colour_map: Option<u32>,
        detail_map: Option<u32>,
        /// Texture and its (u, v) rows over world-space (x, y, z, 1).
        layers: Vec<(u32, [[f32; 4]; 2])>,
    },
    /// Roads, pavements, markings and other pieces laid into the cell,
    /// shaded like models with their own texture coordinates.
    Surface(Material),
}

impl TerrainSurface {
    /// Textures to load, as (hash, holds data rather than colour, is the
    /// cell's colour map).
    pub fn textures(&self) -> Vec<(u32, bool, bool)> {
        match self {
            Self::Ground {
                colour_map,
                detail_map,
                layers,
            } => colour_map
                .map(|t| (t, false, true))
                .into_iter()
                // Signed grain around mid-grey: data, not colour.
                .chain(detail_map.map(|t| (t, true, false)))
                .chain(layers.iter().map(|&(t, _)| (t, false, false)))
                .collect(),
            Self::Surface(m) => crate::model::material_textures(m)
                .into_iter()
                .map(|(t, linear)| (t, linear, false))
                .collect(),
        }
    }
}

/// Terrain geometry in a cell's own space, ready to upload, as a triangle
/// list grouped by what each part is drawn with.
#[derive(Clone, Debug, Default)]
pub struct TerrainMesh {
    /// Where the mesh's origin sits in the world.
    pub origin: Vec3,
    /// Position, normal, texture coordinate, colour and tangent
    /// ([`TerrainMesh::layout`], [`TERRAIN_STRIDE`] bytes per vertex).
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    /// What each range is drawn with.
    pub surfaces: Vec<TerrainSurface>,
    /// (first index, index count, index into `surfaces`), one per surface
    /// used, back to back over `indices`.
    pub ranges: Vec<(u32, u32, usize)>,
    /// Lowest and highest vertex, relative to `origin`.
    pub bounds: (Vec3, Vec3),
    /// The ground surface (without skirts) for collision, relative to
    /// `origin`; built for full-detail cells only.
    pub collision: Option<CollisionShape>,
}

/// Where a vertex's texture coordinate comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum UvSource {
    /// The vertex's own (`TEXCOORD0`).
    Own,
    /// The cell's colour map over its whole square.
    CellMap,
    /// The island-wide map (low-resolution tiles), around `centre`.
    Island,
}

impl TerrainMesh {
    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            (VertexSemantic::Position, VertexFormat::Float3),
            (VertexSemantic::Normal, VertexFormat::Float3),
            (VertexSemantic::TexCoord(0), VertexFormat::Float2),
            (VertexSemantic::Color, VertexFormat::UNorm4x8),
            (VertexSemantic::Tangent, VertexFormat::Float4),
        ])
        .expect("valid layout")
    }

    pub fn vertex_count(&self) -> u32 {
        (self.vertices.len() / TERRAIN_STRIDE) as u32
    }

    /// Vertex positions, relative to `origin`.
    pub fn positions(&self) -> Vec<Vec3> {
        self.vertices
            .chunks_exact(TERRAIN_STRIDE)
            .map(|v| {
                let f = |i: usize| f32::from_le_bytes(v[i..i + 4].try_into().expect("4 bytes"));
                Vec3::new(f(0), f(4), f(8))
            })
            .collect()
    }

    fn build_collision(&mut self) {
        let triangles: Vec<[u32; 3]> = self
            .indices
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]])
            .collect();
        self.collision = CollisionShape::mesh(&self.positions(), &triangles);
    }

    pub fn vertex_desc(&self) -> VertexBufferDesc {
        VertexBufferDesc {
            layout: Self::layout(),
            vertex_count: self.vertex_count(),
            usage: BufferUsage::Static,
        }
    }

    pub fn index_desc(&self) -> IndexBufferDesc {
        IndexBufferDesc {
            format: IndexFormat::U32,
            index_count: self.indices.len() as u32,
            usage: BufferUsage::Static,
        }
    }

    pub fn index_bytes(&self) -> Vec<u8> {
        self.indices.iter().flat_map(|i| i.to_le_bytes()).collect()
    }

    /// Appends `group`'s vertices, offset by `offset` from the mesh origin,
    /// with texture coordinates from `uv`; returns the first one's index.
    fn add_vertices(
        &mut self,
        group: &TerrainGroup<'_>,
        offset: Vec3,
        uv: impl Fn(Vec3, [f32; 2]) -> [f32; 2],
    ) -> u32 {
        let positions = group.positions();
        let read = |usage| {
            group
                .decl
                .find(usage, 0)
                .and_then(|e| group.vertices.read(e))
                .unwrap_or_default()
        };
        let normals = read(DeclUsage::Normal);
        let own_uvs = read(DeclUsage::TexCoord);
        let colours = read(DeclUsage::Color);
        let tangents = read(DeclUsage::Tangent);
        let first = self.vertex_count();
        for (i, p) in positions.iter().enumerate() {
            let pos = Vec3::from(*p) + offset;
            let n = normals
                .get(i)
                .and_then(|n| Vec3::new(n[0], n[1], n[2]).try_normalize())
                .unwrap_or(Vec3::Y);
            self.bounds.0 = self.bounds.0.min(pos);
            self.bounds.1 = self.bounds.1.max(pos);
            let t = uv(pos, own_uvs.get(i).map_or([0.0; 2], |t| [t[0], t[1]]));
            for f in [pos.x, pos.y, pos.z, n.x, n.y, n.z, t[0], t[1]] {
                self.vertices.extend(f.to_le_bytes());
            }
            let c = colours.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]);
            self.vertices
                .extend(c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            let tangent = tangents.get(i).copied().unwrap_or([0.0; 4]);
            for f in tangent {
                self.vertices.extend(f.to_le_bytes());
            }
        }
        first
    }

    /// Adds every triangle of `group`, each with the surface its material
    /// range names (`surface(material index)`), into `buckets` (per
    /// surface, its triangles); full ranges draw it all with `fallback`
    /// when the group has no material ranges.
    #[allow(clippy::too_many_arguments)]
    fn add_group(
        &mut self,
        group: &TerrainGroup<'_>,
        offset: Vec3,
        centre: Vec3,
        surface: impl Fn(u32) -> Option<(usize, UvSource)>,
        fallback: (usize, UvSource),
        buckets: &mut HashMap<usize, Vec<u32>>,
        seen: &mut HashSet<(usize, [u32; 3])>,
    ) {
        let count = group.positions().len();
        let ranges: Vec<_> = if group.material_ranges.is_empty() {
            group.full_ranges.iter().map(|r| (r, fallback)).collect()
        } else {
            group
                .material_ranges
                .iter()
                .map(|r| (r, surface(r.material).unwrap_or(fallback)))
                .collect()
        };
        let mut firsts: HashMap<UvSource, u32> = HashMap::new();
        for (r, (surface, source)) in ranges {
            let first = *firsts.entry(source).or_insert_with(|| {
                self.add_vertices(group, offset, |pos, own| match source {
                    UvSource::Own => own,
                    UvSource::CellMap => colour_map_uv(pos.x, pos.z),
                    UvSource::Island => {
                        let w = pos + centre;
                        [(ISLAND / 2.0 - w.x) / ISLAND, (ISLAND / 2.0 - w.z) / ISLAND]
                    }
                })
            });
            let out = buckets.entry(surface).or_default();
            for tri in group.triangles(r) {
                if tri.iter().any(|&i| usize::from(i) >= count) {
                    continue;
                }
                let tri = tri.map(|i| first + u32::from(i));
                let mut key = tri;
                key.sort_unstable();
                if seen.insert((surface, key)) {
                    out.extend(tri);
                }
            }
        }
    }

    /// Lays `buckets` out as `indices` and `ranges`, by surface.
    fn finish(&mut self, buckets: HashMap<usize, Vec<u32>>) {
        let mut order: Vec<_> = buckets.into_iter().filter(|b| !b.1.is_empty()).collect();
        order.sort_unstable_by_key(|b| b.0);
        for (surface, tris) in order {
            let first = self.indices.len() as u32;
            self.indices.extend(&tris);
            self.ranges.push((first, tris.len() as u32, surface));
        }
    }

    /// Hangs a strip `depth` metres deep below every open edge (an edge of
    /// one triangle only), so cracks where neighbouring cells, patches or
    /// detail levels meet show ground instead of sky. Drawn both ways round,
    /// as the crack may be seen from either side. Only ground ranges get
    /// skirts; they join the first ground range.
    fn add_skirts(&mut self, depth: f32) {
        let ground: Vec<usize> = self
            .ranges
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(self.surfaces[r.2], TerrainSurface::Ground { .. }))
            .map(|(i, _)| i)
            .collect();
        let Some(&host) = ground.first() else {
            return;
        };
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for &g in &ground {
            let (first, count, _) = self.ranges[g];
            for t in self.indices[first as usize..(first + count) as usize].chunks_exact(3) {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    *edges.entry((a.min(b), a.max(b))).or_default() += 1;
                }
            }
        }
        let mut open: Vec<(u32, u32)> = edges
            .into_iter()
            .filter(|&(_, n)| n == 1)
            .map(|(e, _)| e)
            .collect();
        open.sort_unstable();
        let mut lowered: HashMap<u32, u32> = HashMap::new();
        let mut lower = |mesh: &mut Self, v: u32| {
            *lowered.entry(v).or_insert_with(|| {
                let at = v as usize * TERRAIN_STRIDE;
                let mut vertex = mesh.vertices[at..at + TERRAIN_STRIDE].to_vec();
                let y = f32::from_le_bytes(vertex[4..8].try_into().expect("4 bytes")) - depth;
                vertex[4..8].copy_from_slice(&y.to_le_bytes());
                mesh.bounds.0.y = mesh.bounds.0.y.min(y);
                mesh.vertices.extend(vertex);
                mesh.vertex_count() - 1
            })
        };
        let mut skirt = Vec::new();
        for (a, b) in open {
            let (la, lb) = (lower(self, a), lower(self, b));
            skirt.extend([a, b, lb, a, lb, la, a, lb, b, a, la, lb]);
        }
        // Insert after the host range and shift the ranges behind it.
        let (first, count, _) = self.ranges[host];
        let at = (first + count) as usize;
        let added = skirt.len() as u32;
        self.indices.splice(at..at, skirt);
        self.ranges[host].1 += added;
        for r in self.ranges.iter_mut().skip(host + 1) {
            r.0 += added;
        }
    }

    fn empty(origin: Vec3) -> Self {
        Self {
            origin,
            bounds: (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            ..Self::default()
        }
    }
}

/// The surface a cell material draws with: ground for the terrain
/// shaders (or an unknown technique), a model-style surface otherwise.
fn cell_surface(m: &TerrainMaterial) -> TerrainSurface {
    let terrain = Technique::from_hash(m.material.technique).map_or(Some(0), |t| t.terrain_layers);
    match terrain {
        Some(layers) => {
            let slot = |i: usize| m.material.textures.get(i).copied().filter(|&t| t != 0);
            TerrainSurface::Ground {
                colour_map: slot(0),
                detail_map: slot(1),
                layers: (0..usize::from(layers).min(4))
                    .filter_map(|l| {
                        let rows = &m.uv[l];
                        slot(2 + l * 2).map(|t| {
                            (
                                t,
                                [
                                    [rows[0], rows[1], rows[2], rows[3]],
                                    [rows[4], rows[5], rows[6], rows[7]],
                                ],
                            )
                        })
                    })
                    .collect(),
            }
        }
        None => TerrainSurface::Surface(m.material.clone()),
    }
}

/// Reads and decodes terrain cell `hash` centred at `centre`.
pub fn terrain_cell<R: Read + Seek>(
    source: &mut AssetSource<R>,
    hash: u32,
    centre: Vec3,
) -> Result<TerrainMesh, AssetError> {
    let data = source.asset(hash, m2_assets::terrain::TERRAIN_TYPE)?;
    let cell = TerrainCell::parse(&data).map_err(|e| AssetError::Terrain {
        hash,
        message: e.to_string(),
    })?;
    let mut mesh = TerrainMesh::empty(centre);
    mesh.surfaces = cell.materials.iter().map(cell_surface).collect();
    // Groups drawn without a material range use the cell's plain ground.
    let fallback = mesh.surfaces.len();
    mesh.surfaces.push(TerrainSurface::Ground {
        colour_map: cell.colour_map(),
        detail_map: cell
            .materials
            .first()
            .and_then(|m| m.material.textures.get(1).copied())
            .filter(|&t| t != 0),
        layers: Vec::new(),
    });
    let sources: Vec<UvSource> = mesh
        .surfaces
        .iter()
        .map(|s| match s {
            TerrainSurface::Ground { .. } => UvSource::CellMap,
            TerrainSurface::Surface(_) => UvSource::Own,
        })
        .collect();
    let mut buckets = HashMap::new();
    let mut seen = HashSet::new();
    for patch in &cell.patches {
        let offset = Vec3::from(patch.offset);
        for g in &patch.groups {
            mesh.add_group(
                g,
                offset,
                centre,
                |m| sources.get(m as usize).map(|&s| (m as usize, s)),
                (fallback, UvSource::CellMap),
                &mut buckets,
                &mut seen,
            );
        }
    }
    mesh.finish(buckets);
    mesh.build_collision();
    mesh.add_skirts(CELL_SKIRT);
    Ok(mesh)
}

/// Reads and decodes low-resolution tile `hash` centred at `centre`,
/// textured with the island-wide map.
pub fn low_res_tile<R: Read + Seek>(
    source: &mut AssetSource<R>,
    hash: u32,
    centre: Vec3,
) -> Result<TerrainMesh, AssetError> {
    let data = source.asset(hash, m2_assets::terrain::LOW_RES_TYPE)?;
    let tile = LowResTile::parse(&data).map_err(|e| AssetError::Terrain {
        hash,
        message: e.to_string(),
    })?;
    let map = tile
        .materials
        .first()
        .and_then(|m| m.textures.first().copied())
        .filter(|&t| t != 0);
    let mut mesh = TerrainMesh::empty(centre);
    mesh.surfaces.push(TerrainSurface::Ground {
        colour_map: map,
        detail_map: None,
        layers: Vec::new(),
    });
    let mut buckets = HashMap::new();
    // The island map follows the cell maps' axes at island scale.
    mesh.add_group(
        &tile.group,
        Vec3::ZERO,
        centre,
        |_| Some((0, UvSource::Island)),
        (0, UvSource::Island),
        &mut buckets,
        &mut HashSet::new(),
    );
    mesh.finish(buckets);
    mesh.add_skirts(TILE_SKIRT);
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_a_cell_square() {
        let c = Vec3::new(0.0, 5.0, 0.0);
        assert_eq!(square_distance(c, 0.0, 0.0), 0.0);
        assert_eq!(square_distance(c, 199.0, -150.0), 0.0);
        assert_eq!(square_distance(c, 300.0, 0.0), 100.0);
        assert!((square_distance(c, 230.0, 240.0) - 50.0).abs() < 1e-4);
    }

    #[test]
    fn skirts_hang_below_open_edges_only() {
        let mut mesh = TerrainMesh::empty(Vec3::ZERO);
        mesh.surfaces.push(TerrainSurface::Ground {
            colour_map: None,
            detail_map: None,
            layers: Vec::new(),
        });
        // Two triangles sharing the edge 0-2: four open edges.
        for (x, z) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            for f in [x, 2.0, z, 0.0, 1.0, 0.0, 0.0, 0.0] {
                mesh.vertices.extend(f.to_le_bytes());
            }
            mesh.vertices.extend([255, 0, 0, 0]);
            mesh.vertices.extend([0u8; 16]);
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        mesh.ranges = vec![(0, 6, 0)];
        mesh.bounds = (Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.0, 2.0, 1.0));
        mesh.add_skirts(4.0);
        assert_eq!(mesh.vertex_count(), 8);
        assert_eq!(mesh.indices.len(), 6 + 4 * 12);
        assert_eq!(mesh.ranges, vec![(0, 6 + 4 * 12, 0)]);
        assert_eq!(mesh.bounds.0.y, -2.0);
        assert!(mesh.indices.iter().all(|&i| i < 8));
    }

    #[test]
    fn member_names_stem_to_their_model() {
        assert_eq!(
            member_stem("_commercial_bld_corner8x16a 26"),
            "commercial_bld_corner8x16a"
        );
        assert_eq!(
            member_stem("_commercial_bld_segment8x16a 0x001384eb"),
            "commercial_bld_segment8x16a"
        );
        assert_eq!(member_stem("plain"), "plain");
        assert_eq!(member_stem("_"), "");
        assert_eq!(member_stem("AMX30 (Full)"), "AMX30 (Full)");
        assert_eq!(
            member_stem("M151 Softtop (VZ) 0x0012abcd"),
            "M151 Softtop (VZ)"
        );
    }

    #[test]
    fn nearest_cells_first() {
        let mut w = WorldIndex::default();
        w.cells.insert(1, Vec3::new(0.0, 10.0, 0.0));
        w.cells.insert(2, Vec3::new(400.0, 20.0, 0.0));
        w.cells.insert(3, Vec3::new(2000.0, 30.0, 0.0));
        let near: Vec<u32> = w
            .cells_near(250.0, 0.0, 500.0)
            .iter()
            .map(|c| c.0)
            .collect();
        assert_eq!(near, [2, 1]);
        assert_eq!(w.rough_height(1990.0, 0.0), 30.0);
    }
}
