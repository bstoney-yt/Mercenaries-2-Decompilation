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

use m2_assets::layer::{LAYER_TYPE, Placement, TEMPLATE_DB_TYPE};
use m2_assets::model::MODEL_TYPE;
use m2_assets::path::PATH_TYPE;
use m2_assets::terrain::{CELL_SIZE, TerrainGroup, colour_map_uv};
use m2_assets::vertex::DeclUsage;
use m2_assets::{BlockContents, Layer, LowResTile, Path, TerrainCell, name_hash};
use m2_math::glam::{Quat, Vec3};
use m2_render::{
    BufferUsage, IndexBufferDesc, IndexFormat, VertexBufferDesc, VertexFormat, VertexLayout,
    VertexSemantic,
};

use crate::assets::{AssetError, AssetSource};
use crate::physics::CollisionShape;
use crate::pickup::Leavings;
use crate::tuning::VehicleTuning;

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
    /// The name hash of its engine's sound bank (`veh_jeep`), from its
    /// `SoundEffect`.
    pub engine: Option<u32>,
    /// A person (a soldier, a civilian): it has [`PERSON`] physics, or AI
    /// of a person's and no vehicle physics.
    pub person: bool,
    /// It has [`PERSON`] physics: a body that walks (not a spawner of
    /// people, which may have AI).
    pub human: bool,
}

/// The physics component of people, as `_CarPhysicsV2` is of cars.
const PERSON: &str = "_HumanPhysics";
/// AI that only people carry (vehicles have `AiDriving`, `AiHelicopter`).
const PERSON_AI: [&str; 2] = ["AiBehavior", "Ai"];

/// The `SoundEffect` field naming a vehicle's engine sound bank, by name
/// hash (`veh_largegascrappy` for the HMMWVs and Valiants).
const ENGINE_BANK_FIELD: u32 = 0x14a6_7fa6;

/// The template database: what it says of each template by name (lower
/// case), and the names by template id.
#[derive(Clone, Debug, Default)]
pub struct Templates {
    /// By name, lower case: of several so named, the lowest id's.
    pub by_name: HashMap<String, TemplateInfo>,
    /// By id, each its own (several templates may share a name).
    pub infos: HashMap<u32, TemplateInfo>,
    /// Each template's name as the database writes it, by its id (bit 31
    /// set): the id an entity's [`parent`](m2_assets::layer::Placement::parent)
    /// gives.
    pub names: HashMap<u32, String>,
    /// How the templates that say so handle and fight, by id.
    pub tuning: HashMap<u32, VehicleTuning>,
    /// What templates leave when destroyed, and what pickups give.
    pub leavings: Leavings,
    /// How air strikes' ordnance flies and what its blasts do.
    pub ordnance: crate::ordnance::Ordnance,
}

impl Templates {
    /// The template an entity or template name (`M151 Softtop (VZ)
    /// 0x0012abcd`, `AMX30`) names, whatever its case.
    pub fn get(&self, name: &str) -> Option<&TemplateInfo> {
        self.by_name.get(&member_stem(name).to_ascii_lowercase())
    }

    /// The id of the template an entity or template name names, whatever
    /// its case: of several so named, the lowest, so that it is the same
    /// every run.
    pub fn id(&self, name: &str) -> Option<u32> {
        self.ids(name).first().copied()
    }

    /// The ids of every template an entity or template name names,
    /// lowest first.
    pub fn ids(&self, name: &str) -> Vec<u32> {
        let stem = member_stem(name);
        let mut ids: Vec<u32> = self
            .names
            .iter()
            .filter(|(_, n)| n.eq_ignore_ascii_case(stem))
            .map(|(&id, _)| id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// The template with id `id` (an entity's parent): its name, and what
    /// the database says of it.
    pub fn by_id(&self, id: u32) -> Option<(&str, &TemplateInfo)> {
        let name = self.names.get(&id)?;
        let info = match self.infos.get(&id) {
            Some(info) => info,
            None => self.by_name.get(&name.to_ascii_lowercase())?,
        };
        Some((name, info))
    }

    /// The tuning of template `name` (a vehicle's own, `AMX30 (Full)`),
    /// else of a template of its `model`
    /// ([`tuning_for_model`](Self::tuning_for_model)).
    pub fn tuning_for(&self, name: &str, model: u32) -> Option<VehicleTuning> {
        self.ids(name)
            .iter()
            .find_map(|id| self.tuning.get(id).copied())
            .or_else(|| self.tuning_for_model(model))
    }

    /// The tuning of a template of `model`: a model's templates are
    /// variants that share it (the AMX-30's with and without a driver,
    /// for the scripts or the player; an anti-aircraft tank on a tank's
    /// hull). The first by name with a main turret, else with a turret,
    /// else with any.
    pub fn tuning_for_model(&self, model: u32) -> Option<VehicleTuning> {
        let mut ids: Vec<(String, u32)> = self
            .names
            .iter()
            .filter(|(_, n)| self.get(n).is_some_and(|t| t.model == Some(model)))
            .map(|(&id, n)| (n.to_ascii_lowercase(), id))
            .collect();
        ids.sort();
        let tunings: Vec<VehicleTuning> = ids
            .iter()
            .filter_map(|(_, id)| self.tuning.get(id).copied())
            .collect();
        tunings
            .iter()
            .find(|t| t.turret.is_some_and(|t| t.main))
            .or_else(|| tunings.iter().find(|t| t.turret.is_some()))
            .or(tunings.first())
            .copied()
    }

    /// The engine sound bank most templates of `model` use (a model's
    /// templates are variants: drivers, guns, factions).
    pub fn engine_for_model(&self, model: u32) -> Option<u32> {
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for t in self.by_name.values().filter(|t| t.model == Some(model)) {
            if let Some(bank) = t.engine {
                *counts.entry(bank).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|&(bank, n)| (n, std::cmp::Reverse(bank)))
            .map(|(bank, _)| bank)
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
        let engines = engine_banks(&db);
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
        let carrying = |component: &str| -> HashSet<u32> {
            db.component(component)
                .and_then(|c| c.records())
                .unwrap_or_default()
                .into_iter()
                .map(|(id, _)| id)
                .collect()
        };
        let humans = carrying(PERSON);
        let minds: HashSet<u32> = PERSON_AI.iter().flat_map(|c| carrying(c)).collect();
        let names = db.names();
        let infos: HashMap<u32, TemplateInfo> = names
            .keys()
            .map(|&id| {
                let vehicle = kinds.get(&id).copied();
                (
                    id,
                    TemplateInfo {
                        model: model_of.get(&id).copied().filter(|m| models.contains(m)),
                        vehicle,
                        engine: engines.get(&id).copied(),
                        person: humans.contains(&id) || (minds.contains(&id) && vehicle.is_none()),
                        human: humans.contains(&id),
                    },
                )
            })
            .collect();
        // Of twins by name, the lowest id's (inserted last) stands, as in
        // `id`.
        let mut order: Vec<(u32, &str)> = names.iter().map(|(&id, &n)| (id, n)).collect();
        order.sort_unstable_by_key(|&(id, _)| std::cmp::Reverse(id));
        Self {
            by_name: order
                .iter()
                .map(|&(id, name)| (name.to_ascii_lowercase(), infos[&id]))
                .collect(),
            infos,
            names: names
                .into_iter()
                .map(|(id, name)| (id, name.to_owned()))
                .collect(),
            tuning: crate::tuning::read(&db),
            leavings: crate::pickup::read(&db),
            ordnance: crate::ordnance::read(&db),
        }
    }
}

/// Each template's engine sound bank: the `SoundEffect` field that names
/// one, where it isn't zero.
fn engine_banks(db: &Layer<'_>) -> HashMap<u32, u32> {
    let Some(c) = db.component("SoundEffect") else {
        return HashMap::new();
    };
    let Some(at) = c
        .fields
        .iter()
        .find(|f| f.name_hash == ENGINE_BANK_FIELD)
        .map(|f| usize::from(f.offset))
    else {
        return HashMap::new();
    };
    c.records()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, r)| {
            let b = r.get(at..at + 4)?;
            let bank = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            (bank != 0).then_some((id, bank))
        })
        .collect()
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
    /// The entity's name (`VZ Soldier 03`, `vza_beachguard_1`, ...).
    pub name: String,
    /// The name of the template it is an instance of (`VZ Soldier`), if
    /// the database has it: what a renamed entity's name no longer says.
    pub template: Option<String>,
    pub position: Vec3,
    pub rotation: Quat,
    /// The patrol route its `AiPatrol` names, world space; empty for a
    /// post.
    pub route: Vec<Vec3>,
    /// The state layer placing it, by name hash (`vz_state_vzacon001_cp02`):
    /// in the game it is there only while that layer is in. `None` for
    /// the static layers.
    pub layer: Option<u32>,
    /// The model of its template, when that is a person's (`Carmona`'s).
    pub model: Option<u32>,
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
    /// Vehicles the state layers place, by entity, with the template each
    /// is an instance of: what a renamed entity's name
    /// (`VzaCon001_StartingBoat`) no longer says.
    pub state_vehicles: HashMap<u32, TemplatePlacement>,
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
                world.add_people(
                    &layer,
                    e.name_hash,
                    &mut routes,
                    is_static,
                    &assets.templates,
                );
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

    /// The people a layer places: entities with AI or a spawner of their
    /// own, and in a state layer (a mission's), every instance of a person
    /// template (`vza_villageguard_2` carries no components at all).
    fn add_people(
        &mut self,
        layer: &Layer<'_>,
        layer_hash: u32,
        routes: &mut Routes,
        is_static: bool,
        templates: &Templates,
    ) {
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
        let spawn = |id: u32, p: &Placement| {
            let template = templates.by_id(p.parent);
            SpawnPoint {
                entity: id,
                name: names.get(&id).copied().unwrap_or("").to_owned(),
                template: template.map(|(t, _)| t.to_owned()),
                position: Vec3::from(p.position),
                rotation: rotation(p.rotation),
                route: Vec::new(),
                layer: (!is_static).then_some(layer_hash),
                model: template.and_then(|(_, t)| t.model.filter(|_| t.human)),
            }
        };
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
                self.spawns.push(spawn(id, p));
            }
        }
        if is_static {
            return;
        }
        let mut people: Vec<(u32, &Placement)> = placements
            .iter()
            .filter(|(id, p)| {
                !seen.contains(*id)
                    && p.position[1] > HIDDEN_BELOW
                    && templates.by_id(p.parent).is_some_and(|(_, t)| t.person)
            })
            .map(|(&id, p)| (id, p))
            .collect();
        people.sort_unstable_by_key(|&(id, _)| id);
        self.spawns
            .extend(people.into_iter().map(|(id, p)| spawn(id, p)));
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
            self.add_state_vehicles(layer, layer_hash, assets);
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
                            ..
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
            // A prop's template: one with a model, that neither drives
            // nor walks.
            let prop = |t: &TemplateInfo| match t {
                &TemplateInfo {
                    model: Some(m),
                    vehicle: None,
                    person: false,
                    ..
                } => Some(m),
                _ => None,
            };
            let model = models.get(&entity).copied().or_else(|| {
                let h = (!stem.is_empty()).then(|| name_hash(stem));
                if let Some(h) = h.filter(|h| assets.models.contains(h)) {
                    return Some(h);
                }
                // The template its name says, else the one it is an
                // instance of (a renamed entity: `vza001_gate`).
                match assets.templates.get(stem) {
                    Some(t) => prop(t),
                    None => assets.templates.by_id(p.parent).and_then(|(_, t)| prop(t)),
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

    /// A state layer's vehicles: entities whose template, by their name or
    /// else by the one they are an instance of, has vehicle physics.
    fn add_state_vehicles(&mut self, layer: &Layer<'_>, layer_hash: u32, assets: &Assets) {
        let names = layer.names();
        for (&entity, p) in &layer.placements() {
            let stem = names.get(&entity).map_or("", |n| member_stem(n));
            let template = match assets.templates.get(stem) {
                Some(t) => Some((stem, t)),
                None => assets.templates.by_id(p.parent),
            };
            let Some((
                name,
                &TemplateInfo {
                    model,
                    vehicle: Some(kind),
                    ..
                },
            )) = template
            else {
                continue;
            };
            self.state_vehicles.insert(
                entity,
                TemplatePlacement {
                    entity,
                    name: name.to_owned(),
                    position: Vec3::from(p.position),
                    rotation: rotation(p.rotation),
                    model,
                    kind,
                    layer: layer_hash,
                },
            );
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

/// Terrain geometry in a cell's own space, ready to upload: positions,
/// normals and colour-map coordinates, as a triangle list.
#[derive(Clone, Debug, Default)]
pub struct TerrainMesh {
    /// Where the mesh's origin sits in the world.
    pub origin: Vec3,
    /// Position, normal, texture coordinate (32 bytes per vertex).
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    /// The colour map to texture it with.
    pub colour_map: Option<u32>,
    /// The shared detail texture tiled over it up close (cells only).
    pub detail_map: Option<u32>,
    /// Lowest and highest vertex, relative to `origin`.
    pub bounds: (Vec3, Vec3),
    /// The ground surface (without skirts) for collision, relative to
    /// `origin`; built for full-detail cells only.
    pub collision: Option<CollisionShape>,
}

impl TerrainMesh {
    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            (VertexSemantic::Position, VertexFormat::Float3),
            (VertexSemantic::Normal, VertexFormat::Float3),
            (VertexSemantic::TexCoord(0), VertexFormat::Float2),
        ])
        .expect("valid layout")
    }

    pub fn vertex_count(&self) -> u32 {
        (self.vertices.len() / 32) as u32
    }

    /// Vertex positions, relative to `origin`.
    pub fn positions(&self) -> Vec<Vec3> {
        self.vertices
            .chunks_exact(32)
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

    /// Appends `group`, offset by `offset` from the mesh origin, with
    /// texture coordinates from `uv` of the vertex's mesh-space position.
    fn add_group(&mut self, group: &TerrainGroup<'_>, offset: Vec3, uv: impl Fn(Vec3) -> [f32; 2]) {
        let positions = group.positions();
        let normals = group
            .decl
            .find(DeclUsage::Normal, 0)
            .and_then(|e| group.vertices.read(e))
            .unwrap_or_default();
        let first = self.vertex_count();
        for (i, p) in positions.iter().enumerate() {
            let pos = Vec3::from(*p) + offset;
            let n = normals
                .get(i)
                .and_then(|n| Vec3::new(n[0], n[1], n[2]).try_normalize())
                .unwrap_or(Vec3::Y);
            self.bounds.0 = self.bounds.0.min(pos);
            self.bounds.1 = self.bounds.1.max(pos);
            let t = uv(pos);
            for f in [pos.x, pos.y, pos.z, n.x, n.y, n.z, t[0], t[1]] {
                self.vertices.extend(f.to_le_bytes());
            }
        }
        // Full ranges draw the whole group; fall back to the material ranges.
        let ranges = if group.full_ranges.is_empty() {
            &group.material_ranges
        } else {
            &group.full_ranges
        };
        let mut seen = HashSet::new();
        for r in ranges {
            for tri in group.triangles(r) {
                let mut key = tri;
                key.sort_unstable();
                if tri.iter().all(|&i| usize::from(i) < positions.len()) && seen.insert(key) {
                    self.indices.extend(tri.map(|i| first + u32::from(i)));
                }
            }
        }
    }

    /// Hangs a strip `depth` metres deep below every open edge (an edge of
    /// one triangle only), so cracks where neighbouring cells, patches or
    /// detail levels meet show ground instead of sky. Drawn both ways round,
    /// as the crack may be seen from either side.
    fn add_skirts(&mut self, depth: f32) {
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for t in self.indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
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
                let at = v as usize * 32;
                let mut vertex: [u8; 32] = mesh.vertices[at..at + 32].try_into().expect("32 bytes");
                let y = f32::from_le_bytes(vertex[4..8].try_into().expect("4 bytes")) - depth;
                vertex[4..8].copy_from_slice(&y.to_le_bytes());
                mesh.bounds.0.y = mesh.bounds.0.y.min(y);
                mesh.vertices.extend(vertex);
                mesh.vertex_count() - 1
            })
        };
        for (a, b) in open {
            let (la, lb) = (lower(self, a), lower(self, b));
            self.indices
                .extend([a, b, lb, a, lb, la, a, lb, b, a, la, lb]);
        }
    }

    fn empty(origin: Vec3, colour_map: Option<u32>) -> Self {
        Self {
            origin,
            colour_map,
            bounds: (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            ..Self::default()
        }
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
    let mut mesh = TerrainMesh::empty(centre, cell.colour_map());
    // Texture slot 1 of the cell's material, by the format notes.
    mesh.detail_map = cell
        .materials
        .first()
        .and_then(|m| m.material.textures.get(1).copied())
        .filter(|&t| t != 0);
    for patch in &cell.patches {
        let offset = Vec3::from(patch.offset);
        for g in &patch.groups {
            mesh.add_group(g, offset, |p| colour_map_uv(p.x, p.z));
        }
    }
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
    let mut mesh = TerrainMesh::empty(centre, map);
    // The island map follows the cell maps' axes at island scale.
    mesh.add_group(&tile.group, Vec3::ZERO, |p| {
        let w = p + centre;
        [(ISLAND / 2.0 - w.x) / ISLAND, (ISLAND / 2.0 - w.z) / ISLAND]
    });
    mesh.add_skirts(TILE_SKIRT);
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_models_engine_is_the_one_most_of_its_templates_use() {
        let t = |model, engine| TemplateInfo {
            model: Some(model),
            vehicle: Some(VehicleKind::Car),
            engine,
            person: false,
            human: false,
        };
        let db = Templates {
            by_name: HashMap::from([
                ("valiant".to_owned(), t(1, Some(0xaa))),
                ("valiant (driver)".to_owned(), t(1, Some(0xaa))),
                ("valiant (python)".to_owned(), t(1, Some(0xbb))),
                ("valiant (base)".to_owned(), t(1, None)),
                ("hmmwv".to_owned(), t(2, None)),
            ]),
            names: HashMap::from([(0x8000_0001, "Valiant (Python)".to_owned())]),
            ..Templates::default()
        };
        assert_eq!(
            db.by_id(0x8000_0001).map(|(n, _)| n),
            Some("Valiant (Python)")
        );
        assert_eq!(db.by_id(0x8000_0002), None);
        assert_eq!(db.engine_for_model(1), Some(0xaa));
        assert_eq!(db.engine_for_model(2), None);
        assert_eq!(db.engine_for_model(3), None);
    }

    // Round 93's crate rockets gave her nothing: found by name, the RPG's
    // info was another's (likely a second template named "RPG").
    #[test]
    fn same_named_templates_keep_their_own_infos() {
        let t = |model| TemplateInfo {
            model: Some(model),
            vehicle: None,
            engine: None,
            person: false,
            human: false,
        };
        let db = Templates {
            by_name: HashMap::from([("rpg".to_owned(), t(1))]),
            infos: HashMap::from([(0x8000_0001, t(1)), (0x8000_0002, t(2))]),
            names: HashMap::from([
                (0x8000_0001, "RPG".to_owned()),
                (0x8000_0002, "RPG".to_owned()),
            ]),
            ..Templates::default()
        };
        assert_eq!(db.by_id(0x8000_0002).map(|(_, i)| i.model), Some(Some(2)));
        assert_eq!(db.get("RPG").map(|i| i.model), Some(Some(1)));
        assert_eq!(db.id("rpg"), Some(0x8000_0001));
    }

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
        let mut mesh = TerrainMesh::empty(Vec3::ZERO, None);
        // Two triangles sharing the edge 0-2: four open edges.
        for (x, z) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            for f in [x, 2.0, z, 0.0, 1.0, 0.0, 0.0, 0.0] {
                mesh.vertices.extend(f.to_le_bytes());
            }
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        mesh.bounds = (Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.0, 2.0, 1.0));
        mesh.add_skirts(4.0);
        assert_eq!(mesh.vertex_count(), 8);
        assert_eq!(mesh.indices.len(), 6 + 4 * 12);
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
