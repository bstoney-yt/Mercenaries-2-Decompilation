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
use m2_assets::model::Material;
use m2_assets::path::PATH_TYPE;
use m2_assets::terrain::{CELL_SIZE, TerrainGroup, TerrainMaterial, colour_map_uv};
use m2_assets::tree::{CardVertex, TREE_TYPE, TreeTable};
use m2_assets::vertex::DeclUsage;
use m2_assets::{BlockContents, Layer, LowResTile, Path, TerrainCell, name_hash};
use m2_math::glam::{Quat, Vec3};
use m2_render::{
    BufferUsage, IndexBufferDesc, IndexFormat, VertexBufferDesc, VertexFormat, VertexLayout,
    VertexSemantic,
};

use crate::ai::AiProfile;
use crate::assets::{AssetError, AssetSource};
use crate::atmosphere::{Atmosphere, AtmosphereRegion};
use crate::motion::PropMotion;
use crate::physics::CollisionShape;
use crate::pickup::Leavings;
use crate::technique::Technique;
use crate::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable, HandGun, VehicleTuning};

/// Side of the island in metres (20 cells of 400 m).
pub const ISLAND: f32 = 8000.0;

/// Skirt depth under low-resolution tiles. Full-detail cells hang none:
/// with every skirt left out no crack showed between their patches or
/// cells, on steep ground or at coasts, while their skirts stood as
/// smeared walls under banks (sea2) where the seabed drops away.
const TILE_SKIRT: f32 = 30.0;

/// Whether to hang skirts at all (`M2_SKIRTS=0` leaves them out, to see
/// what they cover).
fn skirts_wanted() -> bool {
    static WANTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WANTED.get_or_init(|| std::env::var("M2_SKIRTS").map_or(true, |v| v != "0"))
}

/// With `M2_TERRAIN_NEAR=x,z,r`, prints every triangle of every level of
/// detail whose middle lies within `r` metres of world (x, z): its level,
/// range, material, corners, the y of its face normal (Direct3D winding)
/// and of its vertices' normals.
fn log_near(group: &TerrainGroup<'_>, origin: Vec3) {
    static NEAR: std::sync::OnceLock<Option<(f32, f32, f32)>> = std::sync::OnceLock::new();
    let near = NEAR.get_or_init(|| {
        let v = std::env::var("M2_TERRAIN_NEAR").ok()?;
        let n: Vec<f32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        (n.len() == 3).then(|| (n[0], n[1], n[2]))
    });
    let Some((x, z, r)) = *near else {
        return;
    };
    let positions = group.positions();
    let normals = group
        .decl
        .find(DeclUsage::Normal, 0)
        .and_then(|e| group.vertices.read(e))
        .unwrap_or_default();
    let mut levels: Vec<(String, &m2_assets::terrain::TerrainRange)> = Vec::new();
    for lod in 0..3 {
        for (i, range) in group.lod_ranges(lod).iter().enumerate() {
            levels.push((
                format!("lod {lod} range {i} material {}", range.material),
                range,
            ));
        }
    }
    for (i, range) in group.full_ranges.iter().enumerate() {
        levels.push((format!("full range {i}"), range));
    }
    for (what, range) in levels {
        for t in group.triangles(range) {
            let Some(c) = t
                .iter()
                .map(|&i| {
                    positions
                        .get(usize::from(i))
                        .map(|p| Vec3::from(*p) + origin)
                })
                .collect::<Option<Vec<Vec3>>>()
            else {
                continue;
            };
            let mid = (c[0] + c[1] + c[2]) / 3.0;
            if (mid.x - x).hypot(mid.z - z) > r {
                continue;
            }
            let face = (c[1] - c[0]).cross(c[2] - c[0]).normalize_or_zero();
            let ny: Vec<String> = t
                .iter()
                .map(|&i| {
                    normals
                        .get(usize::from(i))
                        .map_or("-".into(), |n| format!("{:.2}", n[1]))
                })
                .collect();
            eprintln!(
                "near: {what}: [{:.1} {:.1} {:.1}] [{:.1} {:.1} {:.1}] [{:.1} {:.1} {:.1}] face y {:.2}, normals y {}",
                c[0].x,
                c[0].y,
                c[0].z,
                c[1].x,
                c[1].y,
                c[1].z,
                c[2].x,
                c[2].y,
                c[2].z,
                face.y,
                ny.join("/")
            );
        }
    }
}

/// One placed model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectPlacement {
    /// The layer entity placing it.
    pub entity: u32,
    pub model: u32,
    pub position: Vec3,
    pub rotation: Quat,
    /// Placed by its entity's name (`global_env_rocksbeach01 0x00123abc`
    /// is the model `global_env_rocksbeach01`) rather than a `ModelName`:
    /// the island's rocks, plants, fences and sandbags, kept within
    /// [`WorldSettings::prop_distance`](crate::WorldSettings::prop_distance).
    pub named: bool,
    /// Drawn within this many metres, whatever its size: a tree, within
    /// its species' fade distance from the tree table.
    pub fade: Option<f32>,
    /// The template its entity is an instance of (its `Transform`'s
    /// parent), 0 for none.
    pub template: u32,
}

/// Entity ids of the trees: [`TREE_ENTITIES`] | species << 16 | tree. The
/// layers' entity ids stay below `0x2000_0000` (the highest is `0x10001f5b`),
/// so these never meet them.
pub const TREE_ENTITIES: u32 = 0xf000_0000;

/// The species of a tree's entity id, `None` for anything else.
pub fn tree_species(entity: u32) -> Option<usize> {
    (entity & TREE_ENTITIES == TREE_ENTITIES).then_some((entity >> 16 & 0xfff) as usize)
}

/// What stands for a species' tree beyond its fade distance, from the tree
/// table: upright parts turned to face the eye (the trunk and crown, cut
/// from one picture of the tree), and a cap over the crown, seen from
/// above. Inferred, not decoded: that the parts turn about the vertical to
/// face the eye and the cap turns with the tree.
#[derive(Clone, Debug)]
pub struct FarTree {
    pub fade: f32,
    /// Each part's rect (x0, y0, x1, y1, metres, the tree's base at 0) and
    /// its UV rect in the parts' material.
    pub parts: Vec<([f32; 4], [f32; 4])>,
    pub material: Material,
    /// The cap: vertices (tree space), triangle list, material.
    pub cap: Option<(Vec<CardVertex>, Vec<u16>, Material)>,
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

/// The bit every template id has (the database's `UNIQ` ids).
const TEMPLATE_BIT: u32 = 0x8000_0000;

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
    /// The templates whose props move on their own, by id.
    pub motions: HashMap<u32, PropMotion>,
    /// What templates leave when destroyed, and what pickups give.
    pub leavings: Leavings,
    /// Spawners, the lists they spawn from, and patrols' haste.
    pub population: crate::population::Population,
    /// How the person templates fight and notice things, by id.
    pub ai: HashMap<u32, AiProfile>,
    /// How much those with `Health` take, by id.
    pub durability: HashMap<u32, crate::durability::Durability>,
    /// The blasts of those with `Explosive`, by id.
    pub explosions: HashMap<u32, crate::durability::Explosion>,
    /// How the templates that can be hijacked are, by id.
    pub hijack: HashMap<u32, Vec<crate::hijack::HijackEntrance>>,
    /// Where each template's entrances are got in by (`DriverHatch`), by
    /// id ([`crate::hijack::entrance_points`]).
    pub entrance_points: HashMap<u32, Vec<u32>>,
    /// How air strikes' ordnance flies and what its blasts do.
    pub ordnance: crate::ordnance::Ordnance,
    /// How vehicles break each template's objects running into them.
    pub crashable: HashMap<u32, crate::durability::Crash>,
    /// The sound each template's objects make breaking (its `SoundKey`),
    /// by id ([`crate::durability::sound_keys`]).
    pub sound_keys: HashMap<u32, u32>,
    /// The templates that stand already destroyed, by id: those whose
    /// `Health` is 0. All 201 such are ruins (`_marsh_bridgemid_ruined`,
    /// `AMX30_RUIN`), each with the model of its intact twin, whose
    /// `Health` is not 0 (round EC).
    pub ruined: HashSet<u32>,
    /// Each template's `Health`, by id, where it has one.
    pub health: HashMap<u32, f32>,
    /// Each person template's walk, run and sprint speeds, by id
    /// ([`crate::tuning::human_speeds`]).
    pub speeds: HashMap<u32, [f32; 3]>,
    /// Each person template's `HumanAnimationSet` gender and character
    /// name, by id ([`crate::tuning::human_animation_sets`]).
    pub human_sets: HashMap<u32, (u32, u32)>,
    /// Hand-held guns that fire rounds, by id ([`crate::tuning::hand_guns`]).
    pub guns: HashMap<u32, HandGun>,
    /// What each template's `EquipmentLink`s hand it, by id.
    pub equipment: HashMap<u32, Vec<u32>>,
    /// Each template's `LocalizedName` key, by id: the string table's key
    /// for the name the game shows (the `Chopper`'s 1177c0fc reads
    /// "Chopper"; `M151 Softtop (VZ)`'s is `vehicle.m151softtopvz`; PC
    /// data, 2026-10-09).
    pub localized: HashMap<u32, u32>,
    /// The game's damage table, if found.
    pub damage: Option<DamageTable>,
    /// Every gun, thrown weapon and vehicle armament ([`crate::weapons`]).
    pub arms: crate::weapons::Arms,
}

impl Templates {
    /// The template an entity or template name (`M151 Softtop (VZ)
    /// 0x0012abcd`, `AMX30`) names, whatever its case.
    pub fn get(&self, name: &str) -> Option<&TemplateInfo> {
        self.by_name.get(&member_stem(name).to_ascii_lowercase())
    }

    /// How an entity named `name` (its template's name and an id), an
    /// instance of template `parent`, moves on its own: its name's
    /// template's motion, else its parent's.
    pub fn motion(&self, name: &str, parent: u32) -> Option<&PropMotion> {
        let stem = member_stem(name);
        self.motions
            .iter()
            .find(|(id, _)| {
                self.names
                    .get(id)
                    .is_some_and(|n| n.eq_ignore_ascii_case(stem))
            })
            .map(|(_, m)| m)
            .or_else(|| self.motions.get(&parent))
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

    /// Who rides in vehicle template `name` (`M151 (MG) (GR)
    /// (DriverGunner)`): each seat's name and its rider's template name,
    /// as the template's `Rider` records say; empty for an empty vehicle.
    pub fn crew(&self, name: &str) -> Vec<(String, String)> {
        let Some(riders) = self.ids(name).into_iter().find_map(|id| {
            self.population
                .riders
                .get(&id)
                .or_else(|| self.population.riders.get(&(id & !TEMPLATE_BIT)))
        }) else {
            return Vec::new();
        };
        riders
            .iter()
            .filter_map(|&(seat, person)| {
                let (who, _) = self
                    .by_id(person)
                    .or_else(|| self.by_id(person | TEMPLATE_BIT))?;
                Some((crate::population::seat_name(seat), who.to_owned()))
            })
            .collect()
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

    /// The templates a vehicle may be, by its own template `name` (the
    /// mission's `AMX30 (Full)`) and else by its `model`'s templates
    /// (variants that share it), these lower case and by name.
    pub fn names_for(&self, name: &str, model: u32) -> Vec<String> {
        let mut by_model: Vec<String> = self
            .by_name
            .iter()
            .filter(|(_, t)| t.model == Some(model))
            .map(|(n, _)| n.clone())
            .collect();
        by_model.sort();
        let own = self.ids(name);
        own.iter()
            .filter_map(|id| self.names.get(id).cloned())
            .chain(by_model)
            .collect()
    }

    /// The string table keys of the names the game shows for what
    /// [`names_for`](Self::names_for) `name` and `model` give, in that
    /// order: the `LocalizedName`s of those templates that have one.
    pub fn localized_keys(&self, name: &str, model: u32) -> Vec<u32> {
        let mut keys = Vec::new();
        for id in self.names_for(name, model).iter().flat_map(|n| self.ids(n)) {
            if let Some(&k) = self.localized.get(&id)
                && !keys.contains(&k)
            {
                keys.push(k);
            }
        }
        keys
    }

    /// Template `name`'s `Health`, whatever its case: of several so
    /// named, the first that gives one.
    pub fn health(&self, name: &str) -> Option<f32> {
        self.ids(name)
            .iter()
            .find_map(|id| self.health.get(id).copied())
    }

    /// Template `name`'s walk, run and sprint speeds, whatever its case.
    pub fn speeds(&self, name: &str) -> Option<[f32; 3]> {
        self.ids(name)
            .iter()
            .find_map(|id| self.speeds.get(id).copied())
    }

    /// Template `name`'s `HumanAnimationSet` gender and character name
    /// (name hashes), whatever its case.
    pub fn animation_set(&self, name: &str) -> Option<(u32, u32)> {
        self.ids(name)
            .iter()
            .find_map(|id| self.human_sets.get(id).copied())
    }

    /// The rifle template `name` (a hero: `Jen`) is handed by its
    /// `EquipmentLink`s: the first that is a gun firing rounds (Jen's
    /// `Bullpup Rifle`, Mattias's `Assault Rifle`, Chris's `Carbine`; the
    /// RPG fires rockets), with its template's name.
    pub fn own_rifle(&self, name: &str) -> Option<(&str, HandGun)> {
        self.ids(name).iter().find_map(|&id| self.rifle_of(id))
    }

    /// The rifle template id `id`'s `EquipmentLink`s hand it, as
    /// [`own_rifle`](Self::own_rifle) finds it by name.
    pub fn rifle_of(&self, id: u32) -> Option<(&str, HandGun)> {
        let gun = self
            .equipment
            .get(&id)?
            .iter()
            .find_map(|t| self.guns.get(t))?;
        Some((self.names.get(&gun.template)?.as_str(), *gun))
    }

    /// The gun template `name` (`Carbine`), whatever its case.
    pub fn gun(&self, name: &str) -> Option<HandGun> {
        self.ids(name)
            .iter()
            .find_map(|id| self.guns.get(id).copied())
    }

    /// What damage of kind `damage` (`Explosion`) does to a target of
    /// kind `target` (`Hero`), of the damage, by the game's damage table.
    pub fn damage_scale(&self, target: &str, damage: &str) -> Option<f32> {
        self.damage
            .as_ref()?
            .scale(name_hash(target), name_hash(damage))
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

    /// The armament of template `name` (a vehicle's own), else of the
    /// first by name of `model`'s templates whose driver fires something,
    /// else that mounts anything.
    pub fn armament_for(&self, name: &str, model: u32) -> Option<&crate::weapons::Armament> {
        if let Some(a) = self
            .ids(name)
            .iter()
            .find_map(|id| self.arms.armaments.get(id))
        {
            return Some(a);
        }
        let mut ids: Vec<(String, u32)> = self
            .names
            .iter()
            .filter(|(_, n)| self.get(n).is_some_and(|t| t.model == Some(model)))
            .map(|(&id, n)| (n.to_ascii_lowercase(), id))
            .collect();
        ids.sort();
        let found: Vec<&crate::weapons::Armament> = ids
            .iter()
            .filter_map(|(_, id)| self.arms.armaments.get(id))
            .collect();
        let driver = name_hash("seat_driver");
        found
            .iter()
            .find(|a| !a.of_seat(driver).is_empty())
            .or_else(|| found.iter().find(|a| !a.mounts.is_empty()))
            .copied()
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
    /// Reads the one template database asset, if the archive has it, and
    /// the outdoor atmosphere it holds.
    fn read<R: Read + Seek>(
        source: &mut AssetSource<R>,
        models: &HashSet<u32>,
    ) -> (Self, Option<Atmosphere>) {
        let index = source.archive().index();
        let Some(hash) = index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
        else {
            return Default::default();
        };
        let Ok(data) = source.asset(hash, TEMPLATE_DB_TYPE) else {
            return Default::default();
        };
        let Ok(db) = Layer::parse(&data) else {
            return Default::default();
        };
        let atmosphere = Atmosphere::read(&db, "Default");
        let damage = source
            .asset(name_hash(DAMAGE_TABLE), DAMAGE_TABLE_TYPE)
            .ok()
            .and_then(|d| DamageTable::parse(&d));
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
        let templates = Self {
            by_name: order
                .iter()
                .map(|&(id, name)| (name.to_ascii_lowercase(), infos[&id]))
                .collect(),
            infos,
            names: names
                .into_iter()
                .map(|(id, name)| (id, name.to_owned()))
                .collect(),
            tuning: crate::tuning::read(&db, damage.as_ref()),
            leavings: crate::pickup::read(&db),
            ruined: ruined(&db),
            motions: crate::motion::read_templates(&db),
            population: crate::population::Population::read(&db),
            ai: crate::ai::read(&db),
            durability: crate::durability::read(&db),
            explosions: crate::durability::explosions(&db),
            hijack: crate::hijack::read(&db),
            entrance_points: crate::hijack::entrance_points(&db),
            ordnance: crate::ordnance::read(&db),
            crashable: crate::durability::crashable(&db),
            sound_keys: crate::durability::sound_keys(&db),
            health: crate::tuning::healths(&db),
            speeds: crate::tuning::human_speeds(&db),
            human_sets: crate::tuning::human_animation_sets(&db),
            guns: crate::tuning::hand_guns(&db),
            equipment: crate::tuning::equipment(&db),
            arms: crate::weapons::read(&db),
            localized: localized_names(&db),
            damage,
        };
        (templates, atmosphere)
    }
}

/// Each template's `LocalizedName` key (its field `kEy`, a name hash),
/// by id, where it gives one.
fn localized_names(db: &Layer<'_>) -> HashMap<u32, u32> {
    let t = crate::tuning::Table::new(db, "LocalizedName");
    let field = name_hash("kEy");
    t.templates()
        .filter_map(|id| {
            let key = t.word(t.of_template(id).first()?, field)?;
            (key != 0).then_some((id, key))
        })
        .collect()
}

/// The templates whose `Health` is 0 (its first field, an f32).
fn ruined(db: &Layer<'_>) -> HashSet<u32> {
    db.component("Health")
        .and_then(|c| c.records())
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, r)| r.get(..4) == Some(&[0; 4][..]))
        .map(|(id, _)| id)
        .collect()
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

/// A place the game starts or puts the player: the static layers'
/// `<name>_Start<n>` entities, such as each headquarters'
/// `Starter_<hq>_Start1` and each contract's `<contract>_Start1`
/// (`PmcCon001_Start1`, where the scripts put the player after the first
/// contract).
#[derive(Clone, Debug, PartialEq)]
pub struct StartPoint {
    /// The entity's name without its id (`Starter_Pmc_Start1`).
    pub name: String,
    pub position: Vec3,
    pub rotation: Quat,
}

/// A spawner of vehicles a layer places: a `PointTraffic_*` roadblock
/// or a `PathSpawner_*`, which spawns from its list every so often while
/// the player is near; what it spawns drives its patrol path, if it has
/// one, or stands where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct TrafficPoint {
    pub entity: u32,
    /// The entity's name without its id (`PointTraffic_MaracaiboVZ`).
    pub name: String,
    pub position: Vec3,
    pub rotation: Quat,
    /// The layer it is in, by name hash: a state layer, or one of the
    /// static sub-layers `layers_static` holds (`vz_mar_village`), which
    /// the game loads by name too. `None`: no layer known (it stands).
    pub layer: Option<u32>,
    /// Its layer is one of `layers_static`'s, not a mission's state layer.
    pub static_layer: bool,
    pub spawner: crate::population::Spawner,
    /// What it spawns: template names that drive, with their weights.
    pub vehicles: Vec<(String, f32)>,
    /// Its patrol path in the world (the path entity its `AiPatrol`
    /// names, else its own `Path`); empty: it stands.
    pub route: Vec<Vec3>,
    /// `AiPatrol`'s `Haste` (0.175 to 0.35 on the install).
    pub haste: Option<f32>,
}

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
    /// The layer placing it, by name hash: a state layer
    /// (`vz_state_vzacon001_cp02`) or one of the static sub-layers
    /// `layers_static` holds (`vz_mar_village`), which the start-up
    /// scripts load by name too. In the game it is there only while that
    /// layer is in. `None`: no layer known (it stands).
    pub layer: Option<u32>,
    /// Its layer is one of `layers_static`'s, not a mission's state layer.
    pub static_layer: bool,
    /// The model of its template, when that is a person's (`Carmona`'s).
    pub model: Option<u32>,
    /// For a spawner of people (`PathSpawner_GurHQ_Peds`): whom its list
    /// spawns, by template name, with each one's model and weight.
    pub list: Vec<(String, u32, f32)>,
    /// How its template fights, with the entity's own `AiBehavior`
    /// switches when it has them; `None` when the template has no `Ai`.
    pub ai: Option<AiProfile>,
    /// The rifle its template's `EquipmentLink`s hand it
    /// ([`Templates::rifle_of`]); `None` when they hand it none.
    pub gun: Option<HandGun>,
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
    /// Each tree species' far stand-in, by species (the tree table's
    /// order; `None` for a species not placed).
    pub far_trees: Vec<Option<FarTree>>,
    /// The outdoor light and air (ambient cube, sunlight, haze).
    pub atmosphere: Option<Atmosphere>,
    /// The regions whose light differs from it (`vz_atmosphere_regions`).
    pub atmosphere_regions: Vec<AtmosphereRegion>,
    /// How the placed props that move on their own move, by entity (state
    /// layers' and static layers' alike).
    pub motions: HashMap<u32, PropMotion>,
    /// Models the state layers place (blocks of their own, which the
    /// mission scripts load and unload: `vz_state_vzacon001`), by entity:
    /// what stands while the layer is in, such as a mission's gates.
    /// Vehicles and people are left out (the alpha places those itself).
    pub state_objects: HashMap<u32, ObjectPlacement>,
    /// Vehicles the state layers place, by entity, with the template each
    /// is an instance of: what a renamed entity's name
    /// (`VzaCon001_StartingBoat`) no longer says.
    pub state_vehicles: HashMap<u32, TemplatePlacement>,
    /// Those of [`state_objects`](Self::state_objects) whose template
    /// stands destroyed ([`Templates::ruined`]): drawn as their wrecks.
    pub ruined: HashSet<u32>,
    /// With `M2_LOG_FOLIAGE=on` only: each named entity of the static
    /// layers, its name and how it was placed (`m` by `ModelName`, `p` by
    /// its name or template, `v` a vehicle or unknown template, `c` left
    /// out for a component, `a` named for an asset that isn't a model,
    /// `t` a test entity, `-` nothing).
    pub names: HashMap<u32, (String, char, u32)>,
    /// Where the static layers start the player, by name.
    pub starts: Vec<StartPoint>,
    /// Spawners of vehicles the layers place (`PointTraffic_*`,
    /// `PathSpawner_*`).
    pub traffic: Vec<TrafficPoint>,
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

/// `Starter_Pmc_Start1`, `PmcCon001_Start2`: a name ending `_Start<n>`.
fn is_start(name: &str) -> bool {
    name.rsplit_once('_').is_some_and(|(_, last)| {
        last.len() > 5
            && last[..5].eq_ignore_ascii_case("start")
            && last[5..].bytes().all(|b| b.is_ascii_digit())
    })
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
    /// With `M2_LOG_FOLIAGE=on` only: each asset's type, by name hash.
    types: HashMap<u32, u32>,
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
            types: if std::env::var("M2_LOG_FOLIAGE").as_deref() == Ok("on") {
                index
                    .assets()
                    .iter()
                    .map(|a| (a.name_hash, index.type_hash(a)))
                    .collect()
            } else {
                HashMap::new()
            },
            templates: Templates::default(),
        };
        let (templates, atmosphere) = Templates::read(source, &assets.models);
        assets.templates = templates;

        let mut world = Self {
            atmosphere,
            ..Self::default()
        };
        let mut routes = Routes::default();
        // Each road's placement and path, for clearing trees off it.
        let mut roads: Vec<(Vec3, Quat, u32)> = Vec::new();
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
                if let Some(c) = layer.component("Road") {
                    let placements = layer.placements();
                    for (entity, rec) in c.records().unwrap_or_default() {
                        let (Some(p), Some(path)) = (
                            placements.get(&entity),
                            rec.get(..4)
                                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                        ) else {
                            continue;
                        };
                        roads.push((Vec3::from(p.position), rotation(p.rotation), path));
                    }
                }
                if layer.component("AtmosphereBase").is_some() {
                    world
                        .atmosphere_regions
                        .extend(crate::atmosphere::read_regions(&layer, |h| {
                            let data = source.asset(h, m2_assets::region::REGION_TYPE).ok()?;
                            Some(m2_assets::Region::parse(&data).ok()?.corners)
                        }));
                }
                world.add_people(
                    &layer,
                    e.name_hash,
                    &mut routes,
                    is_static,
                    &assets.templates,
                );
                world.add_traffic(&layer, e.name_hash, is_static, &assets.templates);
            }
        }
        let roads = RoadLines::read(source, &roads);
        world.add_trees(source, &assets.templates, &roads);
        world.template_db = assets.templates;
        let mut paths: HashMap<u32, Vec<Vec3>> = HashMap::new();
        let mut path_points = |asset: u32| -> Vec<Vec3> {
            paths
                .entry(asset)
                .or_insert_with(|| {
                    source
                        .asset(asset, PATH_TYPE)
                        .ok()
                        .and_then(|data| Path::parse(&data).ok())
                        .map(|p| p.points.into_iter().map(Vec3::from).collect())
                        .unwrap_or_default()
                })
                .clone()
        };
        // A spawner's route: the path entity its patrol names, else its
        // own path, each in its entity's space.
        for point in &mut world.traffic {
            let owner = routes
                .patrol_of
                .get(&point.entity)
                .copied()
                .unwrap_or(point.entity);
            let (Some(&(at, q)), Some(&asset)) =
                (routes.placed.get(&owner), routes.path_of.get(&owner))
            else {
                continue;
            };
            point.route = path_points(asset).iter().map(|&p| at + q * p).collect();
        }
        for spawn in &mut world.spawns {
            // Its patrol's path entity, else its own path (a path
            // spawner's).
            let target = routes
                .patrol_of
                .get(&spawn.entity)
                .copied()
                .unwrap_or(spawn.entity);
            let (Some(&(at, q)), Some(&asset)) =
                (routes.placed.get(&target), routes.path_of.get(&target))
            else {
                continue;
            };
            spawn.route = path_points(asset).iter().map(|&p| at + q * p).collect();
        }
        Ok(world)
    }

    /// The island's trees, which no layer places: the tree table
    /// ([`m2_assets::tree`]) gives each species' template and the decoded
    /// position and yaw of each of its trees. Each tree stands as its
    /// template's `ModelName`, drawn within its species' fade distance, and
    /// as its species' [`FarTree`] beyond. Inferred, not decoded: that the
    /// model stands upright at the position and at its own size (the
    /// species' scale range has no per-tree value to pick from).
    ///
    /// Trees within [`ROAD_CLEAR`] of a road are left out: the table puts
    /// some there (893 within 3 m island-wide, round DX), which the game's
    /// footage shows clear, with trees along the edges.
    fn add_trees<R: Read + Seek>(
        &mut self,
        source: &mut AssetSource<R>,
        templates: &Templates,
        roads: &RoadLines,
    ) {
        let index = source.archive().index();
        let Some(hash) = index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TREE_TYPE)
            .map(|a| a.name_hash)
        else {
            return;
        };
        let table = match source.asset(hash, TREE_TYPE).map(|d| TreeTable::parse(&d)) {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => return eprintln!("trees {hash:08x}: {e}"),
            Err(e) => return eprintln!("trees {hash:08x}: {e}"),
        };
        let first = self.objects.len();
        let highest = self
            .members
            .keys()
            .chain(self.objects.iter().map(|o| &o.entity))
            .max()
            .copied();
        let mut no_model: Vec<(u32, usize)> = Vec::new();
        let mut cleared = 0usize;
        for (species, s) in table.species.iter().enumerate() {
            let model = templates.by_id(s.template).and_then(|(_, t)| t.model);
            let Some(model) = model.filter(|_| species < 1 << 12 && s.trees.len() <= 1 << 16)
            else {
                no_model.push((s.template, s.trees.len()));
                self.far_trees.push(None);
                continue;
            };
            // Indexed before clearing, so each tree keeps its entity id.
            let before = self.objects.len();
            self.objects.extend(
                s.trees
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| !roads.near(t.position[0], t.position[2], ROAD_CLEAR))
                    .map(|(i, t)| ObjectPlacement {
                        entity: TREE_ENTITIES | (species as u32) << 16 | i as u32,
                        model,
                        position: Vec3::from(t.position),
                        rotation: Quat::from_rotation_y(t.yaw()),
                        named: true,
                        fade: Some(s.fade),
                        template: 0,
                    }),
            );
            cleared += s.trees.len() - (self.objects.len() - before);
            let parts = s.part_count.min(3) as usize;
            self.far_trees.push(Some(FarTree {
                fade: s.fade,
                parts: s.parts[..parts]
                    .iter()
                    .copied()
                    .zip(s.part_uvs[..parts].iter().copied())
                    .collect(),
                material: s.material.clone(),
                cap: s
                    .card_material
                    .clone()
                    .map(|m| (s.card_vertices.clone(), s.card_indices.clone(), m)),
            }));
        }
        eprintln!(
            "trees: {} of {} species placed, {} trees ({} cleared off roads; layer entity ids up to {:#x}); left out for want of a model (template, trees): {:08x?}",
            table.species.len() - no_model.len(),
            table.species.len(),
            self.objects.len() - first,
            cleared,
            highest.unwrap_or(0),
            no_model
        );
    }

    /// The motions of a layer's entities whose template moves on its own,
    /// with the angles their own `Door` records give.
    fn add_motions(
        &mut self,
        layer: &Layer<'_>,
        placements: &HashMap<u32, Placement>,
        templates: &Templates,
    ) {
        if templates.motions.is_empty() {
            return;
        }
        let names = layer.names();
        let hinges = crate::motion::hinges(layer);
        for (&entity, p) in placements {
            let name = names.get(&entity).copied().unwrap_or("");
            let Some(motion) = templates.motion(name, p.parent) else {
                continue;
            };
            let mut motion = motion.clone();
            if let Some(&(node, angle)) = hinges.get(&entity) {
                motion.hinges.retain(|&(n, _)| n != node);
                motion.hinges.push((node, angle));
            }
            self.motions.insert(entity, motion);
        }
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
                // A patrol naming no path (0: the guerrilla HQ's path
                // spawners) leaves its entity's own path to it.
                if let Some(b) = rec.get(8..12)
                    && let path = u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                    && path != 0
                {
                    routes.patrol_of.insert(id, path);
                }
            }
        }
        let names = layer.names();
        let mut seen: HashSet<u32> = HashSet::new();
        let own_spawners = layer
            .component("PopulationSimpleSpawner")
            .map(crate::population::spawners)
            .unwrap_or_default();
        let people_of = |id: u32, p: &Placement| -> Vec<(String, u32, f32)> {
            let population = &templates.population;
            let Some(spawner) = own_spawners
                .get(&id)
                .or_else(|| population.spawners.get(&p.parent))
            else {
                return Vec::new();
            };
            population
                .lists
                .get(&spawner.list)
                .or_else(|| population.lists.get(&(spawner.list | TEMPLATE_BIT)))
                .into_iter()
                .flatten()
                .filter_map(|&(t, w)| {
                    let (name, info) = templates
                        .by_id(t)
                        .or_else(|| templates.by_id(t | TEMPLATE_BIT))?;
                    let model = info.model.filter(|_| info.human)?;
                    Some((name.to_owned(), model, w))
                })
                .collect()
        };
        // An entity's own `AiBehavior` stands in for its template's.
        let behaviour = layer.component("AiBehavior").map(crate::tuning::Table::of);
        let spawn = |id: u32, p: &Placement| {
            let template = templates.by_id(p.parent);
            let ai = templates.ai.get(&p.parent).map(|a| {
                let own = behaviour
                    .as_ref()
                    .and_then(|t| t.of_template(id).first().map(|r| crate::ai::flags(t, r)));
                AiProfile {
                    flags: own.unwrap_or(a.flags),
                    ..*a
                }
            });
            SpawnPoint {
                entity: id,
                name: names.get(&id).copied().unwrap_or("").to_owned(),
                template: template.map(|(t, _)| t.to_owned()),
                position: Vec3::from(p.position),
                rotation: rotation(p.rotation),
                route: Vec::new(),
                layer: Some(layer_hash),
                static_layer: is_static,
                model: template.and_then(|(_, t)| t.model.filter(|_| t.human)),
                list: people_of(id, p),
                ai,
                gun: templates.rifle_of(p.parent).map(|(_, g)| g),
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
        // Entities spawning people by their template's spawner alone (the
        // path spawners).
        let mut spawners: Vec<(u32, &Placement)> = placements
            .iter()
            .filter(|(id, p)| {
                !seen.contains(*id) && templates.population.spawners.contains_key(&p.parent)
            })
            .map(|(&id, p)| (id, p))
            .collect();
        spawners.sort_unstable_by_key(|&(id, _)| id);
        for (id, p) in spawners {
            let s = spawn(id, p);
            if !s.list.is_empty() {
                seen.insert(id);
                self.spawns.push(s);
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

    /// The vehicle spawners a layer places: entities with a
    /// `PopulationSimpleSpawner` of their own or by their template, whose
    /// list holds vehicles that drive.
    fn add_traffic(
        &mut self,
        layer: &Layer<'_>,
        layer_hash: u32,
        is_static: bool,
        templates: &Templates,
    ) {
        let own = layer
            .component("PopulationSimpleSpawner")
            .map(crate::population::spawners)
            .unwrap_or_default();
        let own_haste = layer
            .component("AiPatrol")
            .map(crate::population::haste)
            .unwrap_or_default();
        let population = &templates.population;
        let names = layer.names();
        for (&entity, p) in &layer.placements() {
            let Some(&spawner) = own
                .get(&entity)
                .or_else(|| population.spawners.get(&p.parent))
            else {
                continue;
            };
            // Ids as the database's own (bit 31 set), whichever way a
            // field stores them.
            let list = population
                .lists
                .get(&spawner.list)
                .or_else(|| population.lists.get(&(spawner.list | TEMPLATE_BIT)));
            let vehicles: Vec<(String, f32)> = list
                .into_iter()
                .flatten()
                .filter_map(|&(t, w)| {
                    let (name, info) = templates
                        .by_id(t)
                        .or_else(|| templates.by_id(t | TEMPLATE_BIT))?;
                    matches!(info.vehicle, Some(VehicleKind::Car | VehicleKind::Tank))
                        .then(|| (name.to_owned(), w))
                })
                .collect();
            if vehicles.is_empty() {
                continue;
            }
            self.traffic.push(TrafficPoint {
                entity,
                name: member_stem(names.get(&entity).copied().unwrap_or("")).to_owned(),
                position: Vec3::from(p.position),
                rotation: rotation(p.rotation),
                layer: Some(layer_hash),
                static_layer: is_static,
                spawner,
                vehicles,
                route: Vec::new(),
                haste: own_haste
                    .get(&entity)
                    .or_else(|| population.haste.get(&p.parent))
                    .copied(),
            });
        }
    }

    fn add_layer(&mut self, layer: &Layer<'_>, layer_hash: u32, is_static: bool, assets: &Assets) {
        let placements = layer.placements();
        self.add_motions(layer, &placements, &assets.templates);
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
                        entity,
                        model: *model,
                        position: Vec3::from(p.position),
                        rotation: rotation(p.rotation),
                        named: false,
                        fade: None,
                        template: p.parent,
                    });
                }
            }
            let not_templates: HashSet<u32> = NOT_TEMPLATES
                .iter()
                .filter_map(|c| layer.component(c))
                .flat_map(|c| c.records().unwrap_or_default())
                .map(|(entity, _)| entity)
                .collect();
            let logging = std::env::var("M2_LOG_FOLIAGE").as_deref() == Ok("on");
            for (entity, name) in layer.names() {
                let Some(p) = at(entity) else { continue };
                let (position, rotation) = (Vec3::from(p.position), rotation(p.rotation));
                let mut candidates: Vec<u32> = models.get(&entity).copied().into_iter().collect();
                let stem = member_stem(name);
                if is_start(stem) {
                    self.starts.push(StartPoint {
                        name: stem.to_owned(),
                        position,
                        rotation,
                    });
                }
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
                    if logging {
                        let how = if models.contains_key(&entity) {
                            'm'
                        } else {
                            'c'
                        };
                        self.names.insert(entity, (name.to_owned(), how, 0));
                    }
                    continue;
                }
                let template = assets.templates.get(stem);
                if logging {
                    let how = match (stem_hash, template) {
                        (Some(h), _) if assets.models.contains(&h) => 'p',
                        (_, Some(t))
                            if matches!(t.vehicle, Some(VehicleKind::Car | VehicleKind::Tank)) =>
                        {
                            'v'
                        }
                        (_, Some(t)) if t.model.is_some() => 'p',
                        (Some(h), None) if assets.any.contains(&h) => 'a',
                        (Some(_), None) if is_test(stem) => 't',
                        (Some(_), None) => 'v',
                        _ => '-',
                    };
                    let kind = stem_hash.and_then(|h| assets.types.get(&h)).copied();
                    self.names
                        .insert(entity, (name.to_owned(), how, kind.unwrap_or(0)));
                }
                match (stem_hash, template) {
                    // Its name is a model.
                    (Some(h), _) if assets.models.contains(&h) => {
                        self.objects.push(ObjectPlacement {
                            entity,
                            model: h,
                            position,
                            rotation,
                            named: true,
                            fade: None,
                            template: p.parent,
                        })
                    }
                    // A vehicle template that drives, floats or flies: the
                    // alpha places it.
                    (
                        _,
                        Some(&TemplateInfo {
                            model,
                            vehicle:
                                Some(
                                    kind @ (VehicleKind::Car
                                    | VehicleKind::Tank
                                    | VehicleKind::Boat
                                    | VehicleKind::Helicopter),
                                ),
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
                    // A template with a model and no vehicle physics (a
                    // pickup, an emplacement, a prop): placed as it stands.
                    (
                        _,
                        Some(&TemplateInfo {
                            model: Some(model), ..
                        }),
                    ) => self.objects.push(ObjectPlacement {
                        entity,
                        model,
                        position,
                        rotation,
                        named: true,
                        fade: None,
                        template: p.parent,
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
                let ruined = &assets.templates.ruined;
                if ruined.contains(&p.parent)
                    || assets
                        .templates
                        .id(stem)
                        .is_some_and(|t| ruined.contains(&t))
                {
                    self.ruined.insert(entity);
                }
                self.state_objects.insert(
                    entity,
                    ObjectPlacement {
                        entity,
                        model,
                        position: Vec3::from(p.position),
                        rotation: rotation(p.rotation),
                        named: !models.contains_key(&entity),
                        fade: None,
                        template: p.parent,
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
    /// The start point named `name` (any case).
    pub fn start(&self, name: &str) -> Option<&StartPoint> {
        self.starts
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

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
    /// Triangles turned round to face the way their vertices' normals do.
    pub flipped: usize,
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

    /// Adds the finest level of detail of `group`, each triangle with the
    /// surface its material range names (`surface(material index)`), into
    /// `buckets` (per surface, its triangles); the first full range draws
    /// it all with `fallback` when the group has no material ranges. The
    /// coarser levels lie over the same ground and are left out.
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
        log_near(group, offset + centre);
        let count = group.positions().len();
        let ranges: Vec<_> = if group.material_ranges.is_empty() {
            group
                .full_ranges
                .iter()
                .take(1)
                .map(|r| (r, fallback))
                .collect()
        } else {
            group
                .lod_ranges(0)
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

    /// Lays `buckets` out as `indices` and `ranges`, by surface, each
    /// triangle wound to face the way its vertices' normals do.
    fn finish(&mut self, mut buckets: HashMap<usize, Vec<u32>>) {
        self.orient(&mut buckets);
        let mut order: Vec<_> = buckets.into_iter().filter(|b| !b.1.is_empty()).collect();
        order.sort_unstable_by_key(|b| b.0);
        for (surface, tris) in order {
            let first = self.indices.len() as u32;
            self.indices.extend(&tris);
            self.ranges.push((first, tris.len() as u32, surface));
        }
    }

    /// Turns round the triangles wound against the rest: most of a mesh's
    /// triangles face (by their winding) the way their vertices' normals
    /// point, but some of the strips' triangles come out the other way
    /// round, and culled as back faces they left holes in the ground (the
    /// bank under sea2's lip) that collision, being two-sided, didn't have.
    /// A triangle whose face is nearly edge-on to its normals is left be.
    fn orient(&mut self, buckets: &mut HashMap<usize, Vec<u32>>) {
        let vertex = |v: u32, at: usize| {
            let o = v as usize * TERRAIN_STRIDE + at;
            let f = |i: usize| {
                f32::from_le_bytes(self.vertices[o + i..o + i + 4].try_into().expect("4 bytes"))
            };
            Vec3::new(f(0), f(4), f(8))
        };
        // Cosine between a triangle's face and its mean vertex normal.
        let facing = |t: &[u32]| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|v| vertex(v, 0));
            let face = (b - a).cross(c - a).normalize_or_zero();
            let normal =
                (vertex(t[0], 12) + vertex(t[1], 12) + vertex(t[2], 12)).normalize_or_zero();
            face.dot(normal)
        };
        let mut keys: Vec<usize> = buckets.keys().copied().collect();
        keys.sort_unstable();
        let way: f32 = keys
            .iter()
            .flat_map(|k| buckets[k].chunks_exact(3))
            .map(|t| facing(t).signum())
            .sum::<f32>()
            .signum();
        let mut flips = Vec::new();
        for &k in &keys {
            for (i, t) in buckets[&k].chunks_exact(3).enumerate() {
                if facing(t) * way < -0.2 {
                    flips.push((k, i));
                }
            }
        }
        self.flipped += flips.len();
        for (k, i) in flips {
            buckets
                .get_mut(&k)
                .expect("bucket")
                .swap(i * 3 + 1, i * 3 + 2);
        }
    }

    /// Hangs a strip `depth` metres deep below every open edge (an edge of
    /// one triangle only), so cracks where a low-resolution tile meets
    /// another detail level show ground instead of sky. Drawn both ways
    /// round, as the crack may be seen from either side. Only ground ranges
    /// get skirts; they join the first ground range.
    fn add_skirts(&mut self, depth: f32) {
        if !skirts_wanted() {
            return;
        }
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
    // Materials that draw alike share one range (cells repeat them).
    let same: Vec<usize> = (0..mesh.surfaces.len())
        .map(|i| {
            (0..i)
                .find(|&j| mesh.surfaces[j] == mesh.surfaces[i])
                .unwrap_or(i)
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
                |m| sources.get(m as usize).map(|&s| (same[m as usize], s)),
                (fallback, UvSource::CellMap),
                &mut buckets,
                &mut seen,
            );
        }
    }
    mesh.finish(buckets);
    mesh.build_collision();
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
    fn start_points_end_start_and_a_number() {
        assert!(is_start("Starter_Pmc_Start1"));
        assert!(is_start("PmcCon001_Start1"));
        assert!(is_start("VzaCon001_start12"));
        assert!(!is_start("VzaCon001_StartingBoat"));
        assert!(!is_start("loc_pmc"));
        assert!(!is_start("Start1"));
    }

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
    fn a_vehicles_templates_are_its_own_then_its_models() {
        let t = |model| TemplateInfo {
            model: Some(model),
            vehicle: Some(VehicleKind::Car),
            engine: None,
            person: false,
            human: false,
        };
        let db = Templates {
            by_name: HashMap::from([
                ("amx30 (full)".to_owned(), t(1)),
                ("amx30".to_owned(), t(1)),
                ("m151".to_owned(), t(2)),
            ]),
            names: HashMap::from([(0x8000_0001, "AMX30 (Full)".to_owned())]),
            ..Templates::default()
        };
        assert_eq!(
            db.names_for("amx30 (full)", 1),
            ["AMX30 (Full)", "amx30", "amx30 (full)"]
        );
        assert_eq!(db.names_for("vz_veh_m151", 2), ["m151"]);
        assert!(db.names_for("nothing", 3).is_empty());
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
    fn triangles_wound_against_the_rest_are_turned_round() {
        let mut mesh = TerrainMesh::empty(Vec3::ZERO);
        for (x, z) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            for f in [x, 0.0, z, 0.0, 1.0, 0.0, 0.0, 0.0] {
                mesh.vertices.extend(f.to_le_bytes());
            }
            mesh.vertices.extend([255, 0, 0, 0]);
            mesh.vertices.extend([0u8; 16]);
        }
        // Two triangles one way round, the third the other.
        let buckets = HashMap::from([(0, vec![0, 1, 2, 0, 2, 3, 0, 3, 2])]);
        mesh.finish(buckets);
        assert_eq!(mesh.flipped, 1);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3, 0, 2, 3]);
    }

    #[test]
    fn skirts_hang_below_open_edges_only() {
        // A 1 m quad of two triangles sharing the edge 0-2 (four open
        // edges), its corner at (x0, z0) in cell space.
        let quad = |x0: f32, z0: f32| {
            let mut mesh = TerrainMesh::empty(Vec3::ZERO);
            mesh.surfaces.push(TerrainSurface::Ground {
                colour_map: None,
                detail_map: None,
                layers: Vec::new(),
            });
            add_quad(&mut mesh, x0, z0, 2.0);
            mesh
        };
        fn add_quad(mesh: &mut TerrainMesh, x0: f32, z0: f32, y: f32) {
            let base = mesh.vertex_count();
            for (x, z) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                for f in [x0 + x, y, z0 + z, 0.0, 1.0, 0.0, 0.0, 0.0] {
                    mesh.vertices.extend(f.to_le_bytes());
                }
                mesh.vertices.extend([255, 0, 0, 0]);
                mesh.vertices.extend([0u8; 16]);
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
            mesh.ranges = vec![(0, mesh.indices.len() as u32, 0)];
            mesh.bounds = (
                mesh.bounds.0.min(Vec3::new(x0, y, z0)),
                mesh.bounds.1.max(Vec3::new(x0 + 1.0, y, z0 + 1.0)),
            );
        }
        let mut mesh = quad(0.0, 0.0);
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

/// How far from a road's line trees are cleared (metres). Invented: the
/// road records hold no width; the PC footage shows the mission's dirt
/// roads clear (round DX found decoded trees up to 3.2 m from the
/// line of the road to the tank drop).
const ROAD_CLEAR: f32 = 3.5;

/// The island's road lines on the ground plane, binned for nearness
/// tests.
#[derive(Default)]
struct RoadLines {
    /// Segments (x0, z0, x1, z1) by bin.
    bins: HashMap<(i32, i32), Vec<[f32; 4]>>,
}

/// Side of a [`RoadLines`] bin (metres).
const ROAD_BIN: f32 = 32.0;

impl RoadLines {
    /// Each road's path, placed by its entity's transform.
    fn read<R: Read + Seek>(source: &mut AssetSource<R>, roads: &[(Vec3, Quat, u32)]) -> Self {
        let mut lines = Self::default();
        let mut paths: HashMap<u32, Vec<Vec3>> = HashMap::new();
        for &(at, q, path) in roads {
            let points = paths.entry(path).or_insert_with(|| {
                source
                    .asset(path, PATH_TYPE)
                    .ok()
                    .and_then(|data| Path::parse(&data).ok())
                    .map(|p| p.points.into_iter().map(Vec3::from).collect())
                    .unwrap_or_default()
            });
            for w in points.windows(2) {
                let (a, b) = (at + q * w[0], at + q * w[1]);
                lines.add([a.x, a.z, b.x, b.z]);
            }
        }
        lines
    }

    fn bin(x: f32, z: f32) -> (i32, i32) {
        ((x / ROAD_BIN).floor() as i32, (z / ROAD_BIN).floor() as i32)
    }

    /// Files segment `s` in every bin its box, widened by the clearance,
    /// touches.
    fn add(&mut self, s: [f32; 4]) {
        let pad = ROAD_CLEAR;
        let lo = Self::bin(s[0].min(s[2]) - pad, s[1].min(s[3]) - pad);
        let hi = Self::bin(s[0].max(s[2]) + pad, s[1].max(s[3]) + pad);
        // A runaway segment would fill the map; real ones are short.
        if (hi.0 - lo.0 + 1) * (hi.1 - lo.1 + 1) > 4096 {
            return;
        }
        for bx in lo.0..=hi.0 {
            for bz in lo.1..=hi.1 {
                self.bins.entry((bx, bz)).or_default().push(s);
            }
        }
    }

    /// Whether ground point (`x`, `z`) lies within `d` of a road's line.
    fn near(&self, x: f32, z: f32, d: f32) -> bool {
        let Some(segments) = self.bins.get(&Self::bin(x, z)) else {
            return false;
        };
        segments.iter().any(|s| {
            let (ax, az, bx, bz) = (s[0], s[1], s[2], s[3]);
            let (dx, dz) = (bx - ax, bz - az);
            let len = dx * dx + dz * dz;
            let t = if len > 0.0 {
                (((x - ax) * dx + (z - az) * dz) / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, pz) = (ax + t * dx - x, az + t * dz - z);
            px * px + pz * pz < d * d
        })
    }
}

#[cfg(test)]
mod road_tests {
    use super::*;

    #[test]
    fn finds_points_near_a_road() {
        let mut roads = RoadLines::default();
        roads.add([0.0, 0.0, 100.0, 0.0]);
        assert!(roads.near(50.0, 3.0, ROAD_CLEAR));
        assert!(roads.near(-2.0, 0.0, ROAD_CLEAR));
        assert!(!roads.near(50.0, 5.0, ROAD_CLEAR));
        assert!(!roads.near(110.0, 0.0, ROAD_CLEAR));
    }
}
