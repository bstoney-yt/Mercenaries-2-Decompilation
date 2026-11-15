//! Drivable vehicles: what the island's vehicle models are, how they are
//! built, and how they drive. [`catalogue`] also finds the person models.
//!
//! [`catalogue`] finds the vehicle models in the archive by their texture
//! names, and [`VehicleSpec`] reads one model's wheels, seat and door with
//! the heuristics in `m2_assets::vehicle`. [`Physics`](crate::Physics)
//! drives a spec as a Rapier rigid body on ray-cast wheels: each wheel is a
//! suspension ray, the front axle steers, every wheel drives. Of the parts
//! turning with a wheel only the round ones spin; a fork or fender steers
//! with its wheel without spinning (`m2_assets::vehicle::rolls_with_wheel`).
//!
//! # Provenance
//!
//! Clean-room. The kinds, wheels and markers come from the game's data as
//! `m2_assets::vehicle` describes. The masses, engines, brakes, grip and
//! springs come from each template's `_CarPhysicsV2`
//! ([`VehicleTuning`]); how a wheeled vehicle's
//! springs hang and push, and its gravity factor, are from the Xbox 360
//! code ([`Springs`](crate::tuning::Springs)); how the rest of the data
//! drives it is still the alpha's reading (`physics.rs` says which).

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::model::MODEL_TYPE;
use m2_assets::vehicle::{self, DRIVER_DOOR, DRIVER_SEAT};
use m2_assets::{BlockContents, Model, Ucfx};
use m2_math::glam::{Mat3, Mat4, Quat, Vec3};

use crate::assets::{AssetError, AssetSource, TEXTURE_TYPE};
use crate::tuning::VehicleTuning;

/// A vehicle model the archive holds: (model hash, kind, name), the kind
/// as the texture names give it (`car`, `truck`, `apc`, `tank`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub model: u32,
    pub kind: String,
    pub label: String,
}

/// Ground vehicle kinds offered for driving, in the order to offer them.
pub const DRIVABLE: [&str; 7] = ["car", "truck", "apc", "tank", "bus", "van", "semi"];

/// A person model: one per role, by texture name
/// (`<faction>_hum_<role>_..`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonModel {
    pub model: u32,
    /// `vz_hum_soldier`, `civ_hum_female`, ...
    pub role: String,
    pub soldier: bool,
}

/// The vehicle, person and hand-weapon models the archive holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalogue {
    pub vehicles: Vec<CatalogueEntry>,
    /// One model per role, soldiers and civilians; not the player
    /// characters (`pmc_`).
    pub people: Vec<PersonModel>,
    /// Hand weapons by name (see [`WEAPONS`]): (name, model).
    pub weapons: Vec<(&'static str, u32)>,
}

/// Hand weapons found by texture name: the AK-47 everyone carries, the RPG
/// launcher, its rocket and a frag grenade.
pub const WEAPONS: [(&str, &str); 4] = [
    ("ak47", "global_weapon_ak47"),
    ("rpg", "global_weapon_rpg"),
    ("rocket", "global_weapon_rpgrocket"),
    ("grenade", "global_weapon_fraggrenade"),
];

/// Hand weapons found by their models' own names (the names their
/// templates' `ModelName` hash): the first contract's supply crate's
/// grenade launcher and its 40 mm round, her carbine (the template
/// "Carbine"), the satellite designator (the template "Satellite
/// Designator") and Jennifer's own rifle (the template "Bullpup Rifle";
/// Mattias's "Assault Rifle" is the AK-47).
pub const NAMED_WEAPONS: [(&str, &str); 5] = [
    ("mgl", "global_weapon_mgl"),
    ("40mm", "global_weapon_40mmgrenade"),
    ("m4", "global_weapon_m4"),
    ("detonator", "global_weapon_detonator"),
    ("bullpup", "global_weapon_bullpup"),
];

/// Root node of every human skeleton.
const HUMAN_ROOT: u32 = 0xcbc1_eb51;
/// Model flag of skinned models.
const SKINNED: u32 = 0x800;

/// Every vehicle and person model in the archive, found by texture name.
/// This reads every block, so run it off the main thread.
pub fn catalogue<R: Read + Seek>(source: &mut AssetSource<R>) -> Result<Catalogue, AssetError> {
    let blocks = source.archive().index().blocks().len();
    let mut texture_names: HashMap<u32, String> = HashMap::new();
    // (model, textures, human, small static model's node count)
    let mut model_textures: Vec<(u32, Vec<u32>, bool, Option<usize>)> = Vec::new();
    for b in 0..blocks {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            match e.type_hash {
                TEXTURE_TYPE => {
                    if let Some(n) = Ucfx::parse(data).ok().and_then(|u| u.name()) {
                        texture_names.insert(e.name_hash, n.to_ascii_lowercase());
                    }
                }
                MODEL_TYPE if e.unk_0x08 == 0 => {
                    if let Ok(m) = Model::parse(data) {
                        let human = m.info.flags & SKINNED != 0
                            && m.nodes.first().map(|n| n.name_hash) == Some(HUMAN_ROOT);
                        let b = &m.info.bounds;
                        let size = (0..3).map(|a| b.max[a] - b.min[a]).fold(0.0f32, f32::max);
                        let small = (m.info.flags & SKINNED == 0 && (0.05..1.6).contains(&size))
                            .then_some(m.nodes.len());
                        model_textures.push((
                            e.name_hash,
                            m.materials
                                .iter()
                                .flat_map(|mt| mt.textures.iter().copied())
                                .collect(),
                            human,
                            small,
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Catalogue::default();
    // Weapons: the small model with the most nodes wearing the weapon's
    // texture; the launcher wears the rocket's too, so one model per name.
    for (name, texture) in WEAPONS {
        let prefix = format!("{texture}_");
        let found = model_textures
            .iter()
            .filter(|m| !out.weapons.iter().any(|w| w.1 == m.0))
            .filter_map(|m| Some((m, m.3?)))
            .filter(|(m, _)| {
                m.1.iter()
                    .filter_map(|t| texture_names.get(t))
                    .any(|n| n == texture || n.starts_with(&prefix))
            })
            .max_by_key(|&(_, nodes)| nodes);
        if let Some((m, _)) = found {
            out.weapons.push((name, m.0));
        }
    }
    for (name, asset) in NAMED_WEAPONS {
        let hash = m2_assets::name_hash(asset);
        if model_textures.iter().any(|m| m.0 == hash) {
            out.weapons.push((name, hash));
        }
    }
    for (model, textures, human, _) in model_textures {
        let mut names = textures
            .iter()
            .filter_map(|t| texture_names.get(t).map(String::as_str));
        if human {
            let Some(name) = names.find(|n| n.contains("_hum_")) else {
                continue;
            };
            let words: Vec<&str> = name.split('_').collect();
            if words.len() < 3 || words[0] == "pmc" || name.contains("chicken") {
                continue;
            }
            let role = words[..3].join("_");
            if !out.people.iter().any(|p| p.role == role) {
                out.people.push(PersonModel {
                    model,
                    soldier: words[0] != "civ",
                    role,
                });
            }
        } else if let Some((kind, label)) = vehicle::kind_from_textures(names) {
            out.vehicles.push(CatalogueEntry { model, kind, label });
        }
    }
    out.vehicles
        .sort_by(|a, b| (&a.kind, &a.label, a.model).cmp(&(&b.kind, &b.label, b.model)));
    out.people.sort_by(|a, b| a.role.cmp(&b.role));
    Ok(out)
}

/// One wheel, model space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelSpec {
    pub centre: Vec3,
    pub radius: f32,
    pub width: f32,
    /// On the front axle, which steers.
    pub steers: bool,
}

/// A turret of a vehicle besides its main one, as its template's `Turret`
/// record gives it ([`crate::seats::TurretRecord`]): a jeep's pintle
/// gun, a tank's cupola machine gun, a gunboat's deck guns.
///
/// Provenance: from samples, each seat's template names the turret its
/// rider aims (`ControllerTurret` `TurretName`), and the turret its yaw
/// and pitch bones, limits and speeds (all with yaw about the model's y
/// and pitch about its x, forward z, on every record read). The field
/// names are the exe's reflection strings (`Turret` 0x823EC738,
/// `ControllerTurret` 0x823F1368). Inferred: the rider's aim turns it
/// as the alpha turns the main turret (at its speeds, within its limits);
/// the engine's own turret update is not decoded.
#[derive(Clone, Debug, PartialEq)]
pub struct Aimer {
    pub record: crate::seats::TurretRecord,
    /// The nodes that turn with it (its yaw bone and those under it), and
    /// those that tilt (its pitch bone and those under it).
    pub yaw_nodes: Vec<bool>,
    pub pitch_nodes: Vec<bool>,
    /// Where it turns and tilts about, model space (its bones' origins).
    pub yaw_pivot: Option<Vec3>,
    pub pitch_pivot: Option<Vec3>,
}

impl Aimer {
    /// Turret record `record` on a model of nodes `skeleton` (a prop's: an
    /// emplaced gun's tripod), as [`VehicleSpec`] builds a vehicle's from
    /// its node list: the yaw bone and every node under it turn, the pitch
    /// bone and those under it tilt, each about its bone's rest origin.
    /// `None` when the model has neither bone.
    pub fn of_skeleton(
        record: crate::seats::TurretRecord,
        skeleton: &crate::Skeleton,
    ) -> Option<Self> {
        let index = |hash: u32| skeleton.name_hashes.iter().position(|&n| n == hash);
        let yaw = record.bone_yaw.and_then(index);
        let pitch = record.bone_pitch.and_then(index);
        if yaw.is_none() && pitch.is_none() {
            return None;
        }
        let n = skeleton.parents.len();
        let under = |k: usize, top: Option<usize>| {
            let Some(top) = top else { return false };
            let mut at = Some(k);
            for _ in 0..=n {
                match at {
                    Some(c) if c == top => return true,
                    Some(c) => at = skeleton.parents.get(c).copied().flatten(),
                    None => break,
                }
            }
            false
        };
        let rest = skeleton.model_matrices(&skeleton.rest_pose());
        let origin = |i: Option<usize>| i.and_then(|i| rest.get(i)).map(|m| m.w_axis.truncate());
        Some(Self {
            record,
            yaw_nodes: (0..n).map(|k| under(k, yaw)).collect(),
            pitch_nodes: (0..n).map(|k| under(k, pitch)).collect(),
            yaw_pivot: origin(yaw),
            pitch_pivot: origin(pitch),
        })
    }

    /// Every node's matrix in the model (node to model) with it turned
    /// `yaw` and tilted `pitch`, from the nodes' `rest` matrices.
    pub fn pose(&self, rest: &[Mat4], yaw: f32, pitch: f32) -> Vec<Mat4> {
        rest.iter()
            .enumerate()
            .map(|(i, m)| self.transform(i, yaw, pitch) * *m)
            .collect()
    }

    /// How node `i` moves with it turned `yaw` and tilted `pitch` radians
    /// (up positive), model space.
    pub fn transform(&self, i: usize, yaw: f32, pitch: f32) -> Mat4 {
        let about = |at: Option<Vec3>, r: Mat4| {
            at.map_or(Mat4::IDENTITY, |at| {
                Mat4::from_translation(at) * r * Mat4::from_translation(-at)
            })
        };
        let mut m = Mat4::IDENTITY;
        if self.pitch_nodes.get(i).copied().unwrap_or(false) {
            // Models face +z: a turn about x by -pitch lifts the muzzle.
            m = about(self.pitch_pivot, Mat4::from_rotation_x(-pitch)) * m;
        }
        if self.yaw_nodes.get(i).copied().unwrap_or(false) {
            m = about(self.yaw_pivot, Mat4::from_rotation_y(yaw)) * m;
        }
        m
    }

    /// Turns and tilts `aim` (yaw, pitch) toward `want` by at most its
    /// speeds over `dt` seconds, within its limits. A turret that turns
    /// all the way round takes the short way.
    pub fn step(&self, aim: (f32, f32), want: (f32, f32), dt: f32) -> (f32, f32) {
        let r = &self.record;
        let yaw = if r.full_circle() {
            let mut turn = (want.0 - aim.0).rem_euclid(std::f32::consts::TAU);
            if turn > std::f32::consts::PI {
                turn -= std::f32::consts::TAU;
            }
            aim.0 + turn.clamp(-r.yaw_speed * dt, r.yaw_speed * dt)
        } else {
            let goal = want.0.clamp(r.yaw.0, r.yaw.1);
            aim.0 + (goal - aim.0).clamp(-r.yaw_speed * dt, r.yaw_speed * dt)
        };
        let goal = want.1.clamp(r.pitch.0, r.pitch.1);
        let pitch = aim.1 + (goal - aim.1).clamp(-r.pitch_speed * dt, r.pitch_speed * dt);
        (yaw, pitch)
    }
}

/// How to build and draw one vehicle model.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleSpec {
    pub model: u32,
    pub kind: String,
    pub label: String,
    pub wheels: Vec<WheelSpec>,
    /// Model-space bounds.
    pub bounds: (Vec3, Vec3),
    /// Driver's seat and door, model space: the template's seat data's
    /// ([`VehicleSpec::with_driver_points`]) once tuned, else fixed markers.
    pub seat: Option<Vec3>,
    pub door: Option<Vec3>,
    /// Its template's seats and entrances ([`crate::seats::VehicleSeats`]),
    /// when the data gives them ([`VehicleSpec::with_seating`]).
    pub seating: Option<std::sync::Arc<crate::seats::VehicleSeats>>,
    /// Those entrances whose hardpoint node the model has, with that
    /// node's place, model space.
    pub entrances: Vec<(crate::seats::Entrance, Vec3)>,
    /// Tracked vehicles (tanks, APCs without usable wheels) drive on four
    /// made-up wheels; their road wheels turn with the ground covered.
    pub tracked: bool,
    /// A tracked vehicle's road wheels as drawn: centre and radius, model
    /// space.
    pub road_wheels: Vec<(Vec3, f32)>,
    /// Each node's rest transform (node to model), and its name's hash.
    pub rest: Vec<Mat4>,
    pub node_names: Vec<u32>,
    /// The weapons its riders fire, as its template mounts them
    /// ([`VehicleSpec::armed_with`]).
    pub armed: Vec<crate::weapons::Mounted>,
    /// The wheel each node turns with, if any: an index into `wheels`, or
    /// into `road_wheels` when tracked.
    pub node_wheel: Vec<Option<usize>>,
    /// Whether each node turning with a wheel also spins with it (its
    /// round tyre, rim and hub) rather than only steering with it (a
    /// bike's fork and fender, a suspension arm): see
    /// `m2_assets::vehicle::rolls_with_wheel`. Empty, or a node past
    /// its end: every such node spins.
    pub node_rolls: Vec<bool>,
    /// Mass, kg: its template's `Mass` ([`VehicleSpec::tuned`]), else by
    /// kind, invented. A car's body takes its `PHY2` chassis record's
    /// before the invented one (`Physics::add_vehicle`); measured, the
    /// invented one is left only on the eight catalogue models with
    /// neither (five boats, two helicopters and a tank whose models no
    /// physics template names).
    pub mass: f32,
    /// The nodes that turn with the turret (its mount, every node under
    /// it), if it has one.
    pub turret: Vec<bool>,
    /// Where the turret turns about, model space, when its mount node sits
    /// off the turret (else the mount).
    pub pivot: Option<Vec3>,
    /// The turret gun's muzzle, model space, with the turret straight
    /// ahead: the turret's muzzle marker reaching furthest forward (a
    /// tank's cannon, not its cupola's machine gun), else the front of the
    /// turret part reaching furthest forward (the barrel).
    pub muzzle: Option<Vec3>,
    /// The nodes that tilt with the turret's gun as it aims up or down
    /// (its [`GUN`] node shown intact, every node under it).
    pub gun: Vec<bool>,
    /// Where the gun tilts about (its node's origin, the trunnions),
    /// model space, with the turret straight ahead.
    pub gun_pivot: Option<Vec3>,
    /// Whether the muzzle is on the gun (it tilts with it).
    pub muzzle_on_gun: bool,
    /// How far the gun tilts down and up, radians (its template's, else
    /// invented by kind).
    pub elevation: (f32, f32),
    /// The hull's box, model space: the intact parts off the turret, so
    /// not a tank's gun reaching past its bow or its aerials (the model's
    /// [`bounds`](Self::bounds) when it has no turret).
    pub hull: (Vec3, Vec3),
    /// The turret's box with the turret straight ahead, model space,
    /// without its gun and aerials.
    pub turret_box: Option<(Vec3, Vec3)>,
    /// What its template says of how it handles and fights
    /// ([`VehicleSpec::tuned`]).
    pub tuning: VehicleTuning,
    /// A helicopter's rotors (none for others).
    pub rotors: Vec<RotorSpec>,
    /// Parts that tilt with speed (none but the Osprey's nacelles).
    pub tilts: Vec<TiltSpec>,
    /// A bomb bay its driver drops bombs from (none but the Osprey's).
    pub bomb_bay: Option<BombBay>,
    /// Each node's parent, by index.
    pub node_parents: Vec<Option<u16>>,
    /// Its turrets other than the main one ([`turret`](Self::turret)),
    /// each turned by the seat that names it ([`VehicleSpec::with_seating`]).
    pub aimers: Vec<Aimer>,
    /// What its parts do as they are damaged, if its model says
    /// ([`crate::vehicle_damage`]).
    pub damage: Option<std::sync::Arc<crate::vehicle_damage::DamageModel>>,
    /// Its model's intact chassis body ([`VehicleSpec::with_chassis`]),
    /// which a tank, boat or helicopter takes its inertia and centre of
    /// mass from.
    pub chassis: Option<Chassis>,
    /// The shapes its intact chassis body collides with
    /// ([`VehicleSpec::with_chassis`]), model space: each one's points,
    /// convex radius included. The 360 code builds its body from them
    /// (0x8216A290). Empty without them.
    pub shapes: std::sync::Arc<[Vec<Vec3>]>,
    /// Which of [`shapes`](Self::shapes) a path search doesn't see (their
    /// index): by its flags, a shape takes a vehicle's layer (0x8216C308,
    /// [`crate::collision_layers::VEHICLE`]) and the sweeps see layer 8
    /// but not 9 or 11 ([`crate::collision_layers::PATH_MASK`]). 164 of
    /// the 534 vehicle templates with a hull set have a flags 0x28 shape
    /// in it (layer 11), a HMMWV's or a motorbike's.
    pub path_unseen: std::sync::Arc<[usize]>,
    /// What the game makes of its intact chassis body's record
    /// ([`VehicleSpec::with_chassis`]): its friction and damping (the 360
    /// body builder's, [`crate::physics::BodyMaterial::of`]). `None`
    /// without one.
    pub material: Option<crate::physics::BodyMaterial>,
    /// The shapes its turret's own bodies collide with (the `PHY2` bodies
    /// on its [`turret`](Self::turret) nodes: the 360 builds them as bodies
    /// of their own, 0x8216B910), their vehicle hull sets, model space
    /// with the turret straight ahead. Empty without them.
    pub turret_shapes: std::sync::Arc<[Vec<Vec3>]>,
    /// Its turret's bodies' mass, together, model space (its parts'
    /// own `PHY2` bodies': the AMX-30's turret 3.0 t and gun 3.2 t), if
    /// any: the 360 code makes its turret a body of its own on a hinge
    /// (0x82273F90).
    pub turret_mass: Option<Chassis>,
    /// Its turret's gun on a body of its own (the `PHY2` bodies on its
    /// [`gun`](Self::gun) nodes, when it has a turret body and a
    /// [`gun_pivot`](Self::gun_pivot)): the 360 code hinges it to the
    /// turret's body and drives it to the pitch asked (0x82275974,
    /// 0x82273F90). Its bodies are then not in
    /// [`turret_mass`](Self::turret_mass) or
    /// [`turret_shapes`](Self::turret_shapes).
    pub gun_body: Option<GunBody>,
    /// Its wrecked chassis body ([`WRECKED_CHASSIS`]), which the 360 builds
    /// once the wreck's parts show instead of the intact ones.
    pub wreck: Option<WreckBody>,
}

/// The body of a vehicle's wreck: the `PHY2` body on its wrecked chassis
/// part ([`WRECKED_CHASSIS`]).
///
/// From the 360 code: a body's parts are picked by which show
/// (0x8216B910 takes the last showing part of [`CHASSIS`],
/// [`WRECKED_CHASSIS`] or 0x765CD254 as its root); rebuilt with other parts
/// showing, it drops the bodies on parts now hidden, keeping their mass
/// and linear and angular velocity for what replaces them (0x82184370,
/// from 0x8216C570).
#[derive(Clone, Debug, PartialEq)]
pub struct WreckBody {
    /// Its shapes, as [`VehicleSpec::shapes`] takes the intact one's.
    pub shapes: std::sync::Arc<[Vec<Vec3>]>,
    pub material: crate::physics::BodyMaterial,
}

/// How a vehicle sits on uneven ground ([`VehicleSpec::rest_on`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rest {
    /// The model's origin and turn with its wheels on the ground.
    pub position: Vec3,
    pub rotation: Quat,
    /// How steep the ground under its wheels is, radians.
    pub slope: f32,
    /// How far the ground under its footprint rises above the plane its
    /// wheels rest on (a rock or a kerb under the hull, a ledge), metres.
    pub bumps: f32,
}

/// The node a vehicle's intact chassis body hangs on in its model's
/// `PHY2`. From the 360 code: the part walk (0x8216BC08) puts a body on
/// this node, on 0x75F1F74D or on 0x765CD254 first in its object's list
/// (0x82185DB0 -> 0x821824E8), and that first body is the one the
/// template's `Mass` goes to (0x82182848). From samples: each of the 57
/// tank, boat and helicopter models with a `PHY2` has one body here,
/// shown intact and hidden wrecked, and one on 0x75F1F74D, the other
/// way about.
pub const CHASSIS: u32 = 0x255e_ab53;

/// The node hash of a vehicle's wrecked chassis part, shown once it is
/// wrecked (the other of the two [`CHASSIS`] describes).
pub const WRECKED_CHASSIS: u32 = 0x75f1_f74d;

/// A turret's gun's own body ([`VehicleSpec::gun_body`]): its shapes,
/// model space with the turret straight ahead and the gun level, and
/// its mass.
#[derive(Clone, Debug, PartialEq)]
pub struct GunBody {
    pub shapes: std::sync::Arc<[Vec<Vec3>]>,
    pub mass: Chassis,
}

/// A vehicle's intact chassis body as its model's `PHY2` record gives
/// it, in model space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chassis {
    /// The record's mass, kg: the art's, often far from the template's
    /// (the Mi-35's 333,882 against its `Mass` 20,000).
    pub mass: f32,
    /// The record's inertia, kg m², model axes.
    pub inertia: Mat3,
    pub centre_of_mass: Vec3,
}

/// The shapes the intact chassis body of `model` (whose resident asset is
/// `base`) collides with, model space: each standing one's points, its
/// convex radius included. Empty without a `PHY2` chassis body.
#[allow(clippy::type_complexity)]
fn chassis_shapes(
    model: &Model<'_>,
    base: &[u8],
) -> (
    Vec<(Vec<Vec3>, bool)>,
    bool,
    Option<crate::physics::BodyMaterial>,
) {
    let Some(p) = Ucfx::parse(base)
        .ok()
        .and_then(|u| u.top_level(*b"PHY2").map(<[u8]>::to_vec))
        .and_then(|d| m2_assets::physics::Physics::parse(&d).ok())
    else {
        return (Vec::new(), false, None);
    };
    p.parts
        .iter()
        .find(|q| q.node_hash == CHASSIS && q.body.is_some())
        .and_then(|q| q.body)
        .map_or_else(
            || (Vec::new(), false, None),
            |body| {
                let (shapes, hull) = crate::physics::body_shapes_seen(model, &p, body, false);
                let material = p.bodies.get(body).map(crate::physics::BodyMaterial::of);
                (shapes, hull, material)
            },
        )
}

/// `model`'s wrecked chassis body ([`WreckBody`]), if it has one with
/// shapes.
fn wreck_body(model: &Model<'_>, base: &[u8]) -> Option<WreckBody> {
    let p = Ucfx::parse(base)
        .ok()
        .and_then(|u| u.top_level(*b"PHY2").map(<[u8]>::to_vec))
        .and_then(|d| m2_assets::physics::Physics::parse(&d).ok())?;
    let body = p
        .parts
        .iter()
        .find(|q| q.node_hash == WRECKED_CHASSIS && q.body.is_some())?
        .body?;
    let (shapes, _) = crate::physics::body_shapes_showing(model, &p, body, true);
    (!shapes.is_empty()).then(|| WreckBody {
        shapes: shapes.into(),
        material: crate::physics::BodyMaterial::of(&p.bodies[body]),
    })
}

/// The shapes of `model`'s bodies on its turret's nodes (`turret`, by
/// node), as [`chassis_shapes`] takes a body's.
fn turret_shapes(model: &Model<'_>, base: &[u8], turret: &[bool]) -> Vec<Vec<Vec3>> {
    let Some(p) = Ucfx::parse(base)
        .ok()
        .and_then(|u| u.top_level(*b"PHY2").map(<[u8]>::to_vec))
        .and_then(|d| m2_assets::physics::Physics::parse(&d).ok())
    else {
        return Vec::new();
    };
    p.parts
        .iter()
        .filter(|q| turret.get(q.node as usize).copied().unwrap_or(false))
        .filter_map(|q| q.body)
        .flat_map(|body| crate::physics::body_shapes(model, &p, body).0)
        .collect()
}

/// The mass of the turret parts' `PHY2` bodies of the model whose resident
/// asset is `base`, together, model space (each placed on its part as the
/// chassis's is, [`Chassis::of`]; inertias moved to their common centre of
/// mass). `None` without a turret body with mass.
fn turret_mass(base: &[u8], turret: &[bool]) -> Option<Chassis> {
    let phy2 = Ucfx::parse(base).ok()?.top_level(*b"PHY2")?.to_vec();
    let p = m2_assets::physics::Physics::parse(&phy2).ok()?;
    let parts: Vec<Chassis> = p
        .parts
        .iter()
        .filter(|q| turret.get(q.node as usize).copied().unwrap_or(false))
        .filter_map(|q| {
            let b = p.bodies.get(q.body?)?;
            let usable = !b.is_fixed()
                && b.mass > 0.0
                && b.inertia[0] != f32::MAX
                && b.inertia.iter().all(|v| v.is_finite());
            usable.then(|| {
                let turn = Mat3::from_quat(Quat::from_array(q.rotation).normalize());
                Chassis {
                    mass: b.mass,
                    inertia: turn * Mat3::from_cols_array(&b.inertia) * turn.transpose(),
                    centre_of_mass: turn * Vec3::from(b.centre_of_mass) + Vec3::from(q.position),
                }
            })
        })
        .collect();
    let mass: f32 = parts.iter().map(|c| c.mass).sum();
    if parts.is_empty() || mass <= 0.0 {
        return None;
    }
    let centre = parts
        .iter()
        .map(|c| c.centre_of_mass * c.mass)
        .sum::<Vec3>()
        / mass;
    // Each part's inertia about the common centre: its own, plus its mass
    // x (|d|² 1 - d dᵀ).
    let inertia = parts.iter().fold(Mat3::ZERO, |sum, c| {
        let d = c.centre_of_mass - centre;
        let shift = Mat3::from_diagonal(Vec3::splat(d.length_squared()))
            - Mat3::from_cols(d * d.x, d * d.y, d * d.z);
        sum + c.inertia + shift * c.mass
    });
    Some(Chassis {
        mass,
        inertia,
        centre_of_mass: centre,
    })
}

impl Chassis {
    /// The chassis of the model whose resident asset is `base`, if its
    /// `PHY2` has one with an inertia of its own (not left to its
    /// shapes, `f32::MAX`) and some mass.
    pub fn of(base: &[u8]) -> Option<Self> {
        let phy2 = Ucfx::parse(base).ok()?.top_level(*b"PHY2")?.to_vec();
        let p = m2_assets::physics::Physics::parse(&phy2).ok()?;
        let part = p
            .parts
            .iter()
            .find(|q| q.node_hash == CHASSIS && q.body.is_some())?;
        let b = p.bodies.get(part.body?)?;
        if b.is_fixed() || b.inertia[0] == f32::MAX || !b.inertia.iter().all(|v| v.is_finite()) {
            return None;
        }
        // The body sits on its part, in model space.
        let turn = Mat3::from_quat(Quat::from_array(part.rotation).normalize());
        let inertia = Mat3::from_cols_array(&b.inertia);
        Some(Self {
            mass: b.mass,
            inertia: turn * inertia * turn.transpose(),
            centre_of_mass: turn * Vec3::from(b.centre_of_mass) + Vec3::from(part.position),
        })
    }
}

/// The [`CatalogueEntry::kind`] of boats (a template with `_BoatPhysics`).
pub const BOAT: &str = "boat";

/// The kind of a helicopter: it flies ([`Physics::fly`]) and has no
/// wheels.
///
/// [`Physics::fly`]: crate::Physics::fly
pub const HELICOPTER: &str = "helicopter";

/// A rotor as drawn: the nodes that spin with it (its blades and their
/// blur, [`vehicle::rotors`]), where it spins about and on what axis,
/// model space.
#[derive(Clone, Debug, PartialEq)]
pub struct RotorSpec {
    pub nodes: Vec<bool>,
    pub centre: Vec3,
    pub axis: Vec3,
}

/// A bomb bay: where its bombs leave it, model space, what they do where
/// they land, and how long between one and the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BombBay {
    pub at: Vec3,
    pub blast: Option<crate::weapons::Blast>,
    pub gap: f32,
}

/// Parts that tilt together about a pivot as the vehicle speeds up (the
/// Osprey's nacelles, [`crate::osprey`]): the nodes, the pivot and the
/// axis, model space at rest.
#[derive(Clone, Debug, PartialEq)]
pub struct TiltSpec {
    pub nodes: Vec<bool>,
    pub pivot: Vec3,
    pub axis: Vec3,
}

/// Node name hash of a gun's muzzle marker (the hand weapons have it).
pub const GUN_MUZZLE: u32 = 0xb148_6ef3;

/// The node a turret turns about, in every vehicle that has one: both the
/// intact and the wrecked turret hang from it.
pub const TURRET_MOUNT: u32 = 0xc851_b695;

/// The turret itself, under its mount, with its origin on the turret
/// ring's middle (the M1A1's, the Stryker's).
pub const TURRET: u32 = 0x7707_65e4;

/// The turret's gun, under the turret, with its origin where it tilts:
/// a tank's mantlet and barrel (the M1A1's, the AMX-30's), the Stryker's
/// gun. The wreck's turret has a barrel of its own.
pub const GUN: u32 = 0xcc0e_3f83;

/// A turret's gun its template fires slower than this (shots a minute) is
/// a cannon: the Stryker's 45 a minute, as the AMX-30's, where machine
/// guns fire hundreds.
const CANNON_RATE: f32 = 120.0;

/// The six points a tracked vehicle's tracks touch the ground at, model
/// space, as the 360 code drives it on them (`hp_wheel_fl`, `_fr`, `_rl`,
/// `_rr`, `_ml`, `_mr`, in that order, 0x82900338): `None` unless its
/// model has all six.
pub fn track_hardpoints(model: &Model<'_>) -> Option<[Vec3; 6]> {
    const NAMES: [&str; 6] = [
        "hp_wheel_fl",
        "hp_wheel_fr",
        "hp_wheel_rl",
        "hp_wheel_rr",
        "hp_wheel_ml",
        "hp_wheel_mr",
    ];
    let mut out = [Vec3::ZERO; 6];
    for (o, n) in out.iter_mut().zip(NAMES) {
        *o = Vec3::from(vehicle::marker(model, m2_assets::name_hash(n))?);
    }
    Some(out)
}

/// The four wheels a tracked vehicle drives on: under the corners of its
/// road wheels, where the tracks meet the ground, else under its hull's.
fn stand_ins(road: &[(Vec3, f32)], hull: (Vec3, Vec3)) -> Vec<WheelSpec> {
    let r = 0.45;
    let (lo, hi) = hull;
    let (mut x0, mut x1, mut z0, mut z1) = road.iter().fold(
        (
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ),
        |(a, b, c, d), (p, _)| (a.min(p.x), b.max(p.x), c.min(p.z), d.max(p.z)),
    );
    // Both tracks' wheels, front and back.
    if road.len() < 4 || x1 - x0 < 1.0 || z1 - z0 < 1.0 {
        let c = (lo + hi) * 0.5;
        let (x, z) = ((hi.x - lo.x) * 0.4, (hi.z - lo.z) * 0.35);
        (x0, x1, z0, z1) = (c.x - x, c.x + x, c.z - z, c.z + z);
    }
    [(x0, z1), (x1, z1), (x0, z0), (x1, z0)]
        .into_iter()
        .map(|(x, z)| WheelSpec {
            centre: Vec3::new(x, lo.y + r, z),
            radius: r,
            width: 0.5,
            steers: z == z1,
        })
        .collect()
}

impl VehicleSpec {
    /// Reads `model`'s wheels and markers. `None` for a ground vehicle
    /// whose wheels can't be found.
    pub fn from_model(entry: &CatalogueEntry, model: &Model<'_>) -> Option<Self> {
        let b = &model.info.bounds;
        let bounds = (Vec3::from(b.min), Vec3::from(b.max));
        if !bounds.0.is_finite() || !bounds.1.is_finite() || bounds.0.cmpge(bounds.1).any() {
            return None;
        }
        let found = vehicle::find_wheels(model);
        // Two wheels in line: a motorbike (every one of the island's has
        // its pair found, round 04 of the vehicles thread).
        let bike = found.len() == 2 && found.iter().all(|w| w.centre[0].abs() < 0.15);
        let usable = ((4..=10).contains(&found.len()) || bike)
            && found.iter().all(|w| (0.15..=1.3).contains(&w.radius));
        let boat = entry.kind == BOAT;
        let flies = entry.kind == HELICOPTER;
        // A wheeled APC (the Stryker) drives on its wheels.
        let tracked = entry.kind == "tank" || (entry.kind == "apc" && !usable);
        let n = model.nodes.len();
        let mut node_wheel = vec![None; n];
        // Each found wheel's subtree: every node with one of its roots as
        // an ancestor (or itself).
        let mut mark = |i: usize, roots: &[u16]| {
            for &root in roots {
                for (k, slot) in node_wheel.iter_mut().enumerate() {
                    let mut at = Some(k as u16);
                    for _ in 0..=n {
                        match at {
                            Some(c) if c == root => {
                                *slot = Some(i);
                                break;
                            }
                            Some(c) => at = model.nodes.get(usize::from(c)).and_then(|x| x.parent),
                            None => break,
                        }
                    }
                }
            }
        };
        let mut road_wheels = Vec::new();
        let mut wheels: Vec<WheelSpec> = if boat || flies {
            Vec::new()
        } else if usable && !tracked {
            let (back, front) = found
                .iter()
                .map(|w| w.centre[2])
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), z| {
                    (lo.min(z), hi.max(z))
                });
            // The front axle steers; with more than two axles (the
            // Stryker's four), those ahead of the middle.
            let steering = if found.len() > 4 {
                0.5 * (front + back)
            } else {
                front - 0.3
            };
            for (i, w) in found.iter().enumerate() {
                mark(i, &w.nodes);
            }
            found
                .iter()
                .map(|w| WheelSpec {
                    centre: Vec3::from(w.centre),
                    radius: w.radius,
                    width: w.width,
                    steers: w.centre[2] >= steering,
                })
                .collect()
        } else if tracked {
            for w in found.iter().filter(|w| (0.1..=1.3).contains(&w.radius)) {
                mark(road_wheels.len(), &w.nodes);
                road_wheels.push((Vec3::from(w.centre), w.radius));
            }
            // Stand-ins, once the hull is known.
            Vec::new()
        } else {
            return None;
        };
        // Of the nodes turning with a wheel, only its round parts (tyre,
        // rim, hub) spin; the rest (a bike's fork and fender, a truck's
        // suspension arms) only steer with it (playtest 5: the chopper's
        // front fender and fork are its front tyre's parent and sibling).
        let axles: Vec<(Vec3, f32)> = if tracked {
            road_wheels.clone()
        } else {
            wheels.iter().map(|w| (w.centre, w.radius)).collect()
        };
        let reaches: Vec<Vec<vehicle::AxleReach>> = axles
            .iter()
            .map(|&(c, _)| vehicle::node_reach(model, c.to_array()))
            .collect();
        let node_rolls: Vec<bool> = node_wheel
            .iter()
            .enumerate()
            .map(|(k, w)| {
                let Some(w) = *w else { return true };
                reaches
                    .get(w)
                    .and_then(|r| r.iter().find(|a| usize::from(a.node) == k))
                    .is_none_or(|a| vehicle::rolls_with_wheel(a, axles[w].1))
            })
            .collect();
        let rest = (0..n)
            .map(|i| model.node_to_model(i as u16).unwrap_or(Mat4::IDENTITY))
            .collect();
        let mount = model.nodes.iter().position(|x| x.name_hash == TURRET_MOUNT);
        let turret: Vec<bool> = (0..n)
            .map(|k| {
                let mut at = Some(k as u16);
                for _ in 0..=n {
                    match at {
                        Some(c) if Some(usize::from(c)) == mount => return true,
                        Some(c) => at = model.nodes.get(usize::from(c)).and_then(|x| x.parent),
                        None => break,
                    }
                }
                false
            })
            .collect();
        let on_turret = |node: u16| turret.get(usize::from(node)).copied().unwrap_or(false);
        let boxes = if mount.is_some() {
            vehicle::node_boxes(model)
        } else {
            Vec::new()
        };
        // The turret part's own node sits where it turns (the M1A1's and
        // the Stryker's); some mounts sit away from it (the Stryker's is
        // at the hull's origin, off the ring). Without that node, a mount
        // off the turret turns it about the middle of its biggest part.
        let ring = model
            .nodes
            .iter()
            .enumerate()
            .find(|(i, n)| n.name_hash == TURRET && on_turret(*i as u16))
            .and_then(|(i, _)| model.node_to_model(i as u16))
            .map(|m| m.w_axis.truncate());
        let pivot = ring.or_else(|| {
            mount.and_then(|m| {
                let at = model.node_to_model(m as u16)?.w_axis.truncate();
                let b = boxes.iter().filter(|b| on_turret(b.node)).max_by(|a, b| {
                    let area = |b: &vehicle::NodeBox| (b.max[0] - b.min[0]) * (b.max[2] - b.min[2]);
                    area(a).total_cmp(&area(b))
                })?;
                let inside =
                    (b.min[0]..=b.max[0]).contains(&at.x) && (b.min[2]..=b.max[2]).contains(&at.z);
                (!inside).then(|| {
                    Vec3::new(
                        (b.min[0] + b.max[0]) * 0.5,
                        at.y,
                        (b.min[2] + b.max[2]) * 0.5,
                    )
                })
            })
        });
        // Of the turret's muzzles shown intact (not the wreck's), the one
        // reaching furthest forward: a tank's cannon, not the machine gun
        // on its commander's cupola.
        let hidden = model
            .states
            .as_ref()
            .map(|s| s.hidden(&model.nodes, false))
            .unwrap_or_default();
        let muzzle = model
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                n.name_hash == GUN_MUZZLE
                    && on_turret(*i as u16)
                    && !hidden.get(*i).copied().unwrap_or(false)
            })
            .filter_map(|(i, _)| Some((i, model.node_to_model(i as u16)?.w_axis.truncate())))
            .max_by(|a, b| a.1.z.total_cmp(&b.1.z));
        // The gun: the intact one, on the turret.
        let gun_node = model.nodes.iter().enumerate().position(|(i, n)| {
            n.name_hash == GUN && on_turret(i as u16) && !hidden.get(i).copied().unwrap_or(false)
        });
        let under = |k: usize, top: usize| {
            let mut at = Some(k as u16);
            for _ in 0..=n {
                match at {
                    Some(c) if usize::from(c) == top => return true,
                    Some(c) => at = model.nodes.get(usize::from(c)).and_then(|x| x.parent),
                    None => break,
                }
            }
            false
        };
        let gun: Vec<bool> = (0..n)
            .map(|k| gun_node.is_some_and(|g| under(k, g)))
            .collect();
        let gun_pivot = gun_node
            .and_then(|g| model.node_to_model(g as u16))
            .map(|m| m.w_axis.truncate());
        let muzzle_on_gun = muzzle.is_some_and(|(i, _)| gun.get(i).copied().unwrap_or(false));
        // The hull: the intact parts off the turret. The turret's box:
        // its intact parts but the gun and anything as thin as an aerial.
        let intact = |node: u16| !hidden.get(usize::from(node)).copied().unwrap_or(false);
        let thin = |b: &vehicle::NodeBox| (b.max[0] - b.min[0]).max(b.max[2] - b.min[2]) < 0.2;
        let union = |boxes: &mut dyn Iterator<Item = &vehicle::NodeBox>| {
            boxes.fold(None, |acc: Option<(Vec3, Vec3)>, b| {
                let (lo, hi) = (Vec3::from(b.min), Vec3::from(b.max));
                Some(acc.map_or((lo, hi), |(a, z)| (a.min(lo), z.max(hi))))
            })
        };
        let hull = union(
            &mut boxes
                .iter()
                .filter(|b| !on_turret(b.node) && intact(b.node)),
        )
        .filter(|(lo, hi)| lo.is_finite() && hi.is_finite() && (*hi - *lo).min_element() > 0.3)
        .unwrap_or(bounds);
        let turret_box = union(&mut boxes.iter().filter(|b| {
            on_turret(b.node)
                && intact(b.node)
                && !gun.get(usize::from(b.node)).copied().unwrap_or(false)
                && !thin(b)
        }))
        .filter(|(lo, hi)| (*hi - *lo).min_element() > 0.2);
        if tracked {
            // On its six track hardpoints, as the 360 code drives it
            // (0x821A4A60), where its model has them (every tracked
            // template's does; tanktest), else on stand-ins.
            wheels = match track_hardpoints(model) {
                Some(h) => h
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| WheelSpec {
                        centre: p + Vec3::Y * 0.45,
                        radius: 0.45,
                        width: 0.5,
                        steers: i < 2,
                    })
                    .collect(),
                None => stand_ins(&road_wheels, hull),
            };
        }
        let muzzle = muzzle.map(|(_, at)| at).or_else(|| {
            let b = boxes
                .iter()
                .filter(|b| on_turret(b.node))
                .max_by(|a, b| a.max[2].total_cmp(&b.max[2]))?;
            Some(Vec3::new(
                (b.min[0] + b.max[0]) * 0.5,
                (b.min[1] + b.max[1]) * 0.5,
                b.max[2],
            ))
        });
        let elevation = if entry.kind == "tank" {
            (-8f32.to_radians(), 20f32.to_radians())
        } else {
            (-10f32.to_radians(), 45f32.to_radians())
        };
        let marker = |h| vehicle::marker(model, h).map(Vec3::from);
        let mass = match entry.kind.as_str() {
            "car" => 1400.0,
            "truck" | "van" => 3500.0,
            "bus" | "semi" => 9000.0,
            "apc" => 12_000.0,
            "tank" => 40_000.0,
            _ => 2000.0,
        };
        Some(Self {
            model: entry.model,
            kind: entry.kind.clone(),
            label: entry.label.clone(),
            node_names: model.nodes.iter().map(|x| x.name_hash).collect(),
            armed: Vec::new(),
            wheels,
            bounds,
            seat: marker(DRIVER_SEAT),
            door: marker(DRIVER_DOOR),
            seating: None,
            entrances: Vec::new(),
            tracked,
            road_wheels,
            rest,
            node_wheel,
            node_rolls,
            mass,
            turret,
            pivot,
            muzzle,
            gun,
            gun_pivot,
            muzzle_on_gun,
            elevation,
            hull,
            turret_box,
            tuning: VehicleTuning::default(),
            damage: crate::vehicle_damage::DamageModel::from_model(model).map(std::sync::Arc::new),
            chassis: None,
            shapes: Default::default(),
            material: None,
            path_unseen: Default::default(),
            turret_shapes: Default::default(),
            turret_mass: None,
            gun_body: None,
            wreck: None,
            rotors: if flies {
                vehicle::rotors(model)
                    .into_iter()
                    .map(|r| RotorSpec {
                        nodes: (0..n as u16).map(|i| r.nodes.contains(&i)).collect(),
                        centre: Vec3::from(r.centre),
                        axis: Vec3::from(r.axis),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            tilts: Vec::new(),
            bomb_bay: None,
            node_parents: model.nodes.iter().map(|x| x.parent).collect(),
            aimers: Vec::new(),
        })
    }

    /// With the weapons its template's armament `arm` mounts, each at
    /// its hardpoints' nodes.
    pub fn armed_with(
        mut self,
        arm: Option<&crate::weapons::Armament>,
        arms: &crate::weapons::Arms,
    ) -> Self {
        self.armed = arm.map_or_else(Vec::new, |a| {
            crate::weapons::mounted(a, arms, &self.node_names)
        });
        self
    }

    /// With its driver's seat and door where its template's seat data
    /// puts them ([`crate::seats`]), at those hardpoints' nodes. A point
    /// the data doesn't give, or whose node the model lacks, keeps the
    /// fixed model marker (Invented stand-in: those markers are the front
    /// right seat and door, see [`crate::seats`]).
    pub fn with_driver_points(mut self, points: Option<crate::seats::DriverPoints>) -> Self {
        let Some(p) = points else {
            return self;
        };
        let node = |hash: u32| {
            let i = self.node_names.iter().position(|&n| n == hash)?;
            self.rest.get(i).map(|m| m.w_axis.truncate())
        };
        let (seat, door) = (node(p.seat), p.door.and_then(node));
        self.seat = seat.or(self.seat);
        self.door = door.or(self.door);
        self
    }

    /// With its template's seats and entrances, each entrance placed at its
    /// hardpoint's node (one whose node the model lacks is left out).
    pub fn with_seating(
        mut self,
        seating: Option<std::sync::Arc<crate::seats::VehicleSeats>>,
    ) -> Self {
        let node = |hash: u32| {
            let i = self.node_names.iter().position(|&n| n == hash)?;
            self.rest.get(i).map(|m| m.w_axis.truncate())
        };
        self.entrances = seating.as_ref().map_or_else(Vec::new, |s| {
            s.entrances
                .iter()
                .filter_map(|e| Some((*e, node(e.hardpoint?)?)))
                .collect()
        });
        self.aimers = seating
            .as_ref()
            .map_or_else(Vec::new, |s| self.aimers_of(&s.turrets));
        self.seating = seating;
        self
    }

    /// Its `Turret` records other than the main turret's (whose yaw bone
    /// is the [`TURRET`] node the alpha already turns), as [`Aimer`]s on
    /// its model's nodes; a record none of whose bones the model has is
    /// left out.
    fn aimers_of(&self, records: &[crate::seats::TurretRecord]) -> Vec<Aimer> {
        let index = |hash: u32| self.node_names.iter().position(|&n| n == hash);
        let n = self.node_names.len();
        let under = |k: usize, top: Option<usize>| {
            let Some(top) = top else { return false };
            let mut at = Some(k as u16);
            for _ in 0..=n {
                match at {
                    Some(c) if usize::from(c) == top => return true,
                    Some(c) => at = self.node_parents.get(usize::from(c)).copied().flatten(),
                    None => break,
                }
            }
            false
        };
        let depth = |i: Option<usize>| {
            let mut d = 0;
            let mut at = i.map(|i| i as u16);
            while let Some(c) = at {
                d += 1;
                at = self.node_parents.get(usize::from(c)).copied().flatten();
                if d > n {
                    break;
                }
            }
            d
        };
        // The deepest first, so one mounted on another (a gun on a ring)
        // is moved on its mount's rest place before its mount moves it.
        let mut records: Vec<&crate::seats::TurretRecord> = records.iter().collect();
        records
            .sort_by_key(|r| std::cmp::Reverse(depth(r.bone_yaw.or(r.bone_pitch).and_then(index))));
        records
            .into_iter()
            .filter(|r| r.bone_yaw != Some(TURRET))
            .filter_map(|r| {
                let yaw = r.bone_yaw.and_then(index);
                let pitch = r.bone_pitch.and_then(index);
                if yaw.is_none() && pitch.is_none() {
                    return None;
                }
                let origin = |i: Option<usize>| {
                    i.and_then(|i| self.rest.get(i))
                        .map(|m| m.w_axis.truncate())
                };
                Some(Aimer {
                    record: *r,
                    yaw_nodes: (0..n).map(|k| under(k, yaw)).collect(),
                    pitch_nodes: (0..n).map(|k| under(k, pitch)).collect(),
                    yaw_pivot: origin(yaw),
                    pitch_pivot: origin(pitch),
                })
            })
            .collect()
    }

    /// The aimer seat `seat` (its `SeatLink` name) turns, by index into
    /// [`aimers`](Self::aimers).
    pub fn aimer_of_seat(&self, seat: u32) -> Option<usize> {
        let s = self.seating.as_ref()?;
        let name = s.seats.get(s.seat(seat)?)?.turret?;
        self.aimers.iter().position(|a| a.record.name == name)
    }

    /// Whether the main turret is turned from seat `seat` (`None`: from
    /// the driver's seat, as without seat data): the seat whose
    /// `ControllerTurret` names the record whose yaw bone is [`TURRET`];
    /// with no such record (or no seat data), the driver's.
    pub fn turns_main_turret(&self, seat: Option<u32>) -> bool {
        let Some(s) = self.seating.as_deref() else {
            return true;
        };
        let main = s
            .turrets
            .iter()
            .find(|r| r.bone_yaw == Some(TURRET))
            .map(|r| r.name);
        let index = match seat {
            Some(n) => s.seat(n),
            None => s.driver(),
        };
        let Some(i) = index else {
            return seat.is_none();
        };
        match main {
            Some(m) => s.seats[i].turret == Some(m),
            None => s.seats[i].driver,
        }
    }

    /// Where node `i` is, model space, with the main turret turned
    /// `turret` and each aimer at `aims` (yaw, pitch).
    pub fn node_place(&self, i: usize, turret: f32, aims: &[(f32, f32)]) -> Option<Mat4> {
        let mut m = *self.rest.get(i)?;
        for (a, &(yaw, pitch)) in self.aimers.iter().zip(aims) {
            m = a.transform(i, yaw, pitch) * m;
        }
        if self.turret.get(i).copied().unwrap_or(false) {
            m = self.turret_transform(turret) * m;
        }
        Some(m)
    }

    /// Whether aimer `k` sits on the main turret (a tank's cupola gun),
    /// so turns with it too.
    fn aimer_on_turret(&self, k: usize) -> bool {
        let a = &self.aimers[k];
        (0..self.turret.len()).any(|i| {
            self.turret[i]
                && (a.yaw_nodes.get(i).copied().unwrap_or(false)
                    || a.pitch_nodes.get(i).copied().unwrap_or(false))
        })
    }

    /// The yaw and pitch, radians, that lay aimer `k` on `goal` (world
    /// space) with the vehicle at `body` and its main turret turned
    /// `turret`: about its yaw bone from straight ahead (model +z), then
    /// up from level about its pitch bone, as [`Aimer::transform`] moves
    /// them.
    pub fn aim_toward(&self, k: usize, body: Mat4, turret: f32, goal: Vec3) -> (f32, f32) {
        let a = &self.aimers[k];
        let frame = if self.aimer_on_turret(k) {
            body * self.turret_transform(turret)
        } else {
            body
        };
        let local = frame.inverse().transform_point3(goal);
        let yaw = a.yaw_pivot.map_or(0.0, |p| {
            let d = local - p;
            d.x.atan2(d.z)
        });
        // As seen with the mount turned back straight ahead.
        let unturned = a
            .yaw_pivot
            .map_or(local, |p| p + Quat::from_rotation_y(-yaw) * (local - p));
        let pitch = a.pitch_pivot.or(a.yaw_pivot).map_or(0.0, |p| {
            let d = unturned - p;
            d.y.atan2(d.with_y(0.0).length())
        });
        (yaw, pitch)
    }

    /// Which way the gun at node `node` points, model space, when one of
    /// its aimers moves it: straight ahead turned and tilted as that
    /// aimer's `aims`, turned with the main turret too when on it. `None`:
    /// no aimer moves that node.
    pub fn barrel_of(&self, node: usize, turret: f32, aims: &[(f32, f32)]) -> Option<Vec3> {
        let k = self.aimers.iter().position(|a| {
            a.yaw_nodes.get(node).copied().unwrap_or(false)
                || a.pitch_nodes.get(node).copied().unwrap_or(false)
        })?;
        let (yaw, pitch) = aims.get(k).copied().unwrap_or((0.0, 0.0));
        let d = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-pitch) * Vec3::Z;
        Some(if self.aimer_on_turret(k) {
            self.turret_transform(turret).transform_vector3(d)
        } else {
            d
        })
    }

    /// The place, model space, of its entrance named `name`.
    pub fn entrance_at(&self, name: u32) -> Option<Vec3> {
        self.entrances
            .iter()
            .find(|(e, _)| e.name == name)
            .map(|&(_, at)| at)
    }

    /// With its template's tuning: its mass and how far its gun tilts.
    pub fn tuned(mut self, tuning: Option<VehicleTuning>) -> Self {
        let Some(t) = tuning else {
            return self;
        };
        self.tuning = t;
        if let Some(m) = t.mass {
            self.mass = m;
        }
        if let Some(turret) = t.turret {
            self.elevation = turret.pitch;
        }
        self
    }

    /// With its model's intact chassis body ([`Chassis::of`] its resident
    /// asset `base`) and that body's shapes (`model`'s).
    pub fn with_chassis(mut self, model: &Model<'_>, base: &[u8]) -> Self {
        self.chassis = Chassis::of(base);
        let (shapes, hull, material) = chassis_shapes(model, base);
        self.material = material;
        // The gun on a body of its own when its parts have one, hinged to
        // the turret's (0x82275974); the turret's body then without it.
        let gun_shapes = turret_shapes(model, base, &self.gun);
        self.gun_body = match turret_mass(base, &self.gun) {
            Some(mass) if !gun_shapes.is_empty() && self.gun_pivot.is_some() => Some(GunBody {
                shapes: gun_shapes.into(),
                mass,
            }),
            _ => None,
        };
        let turret: Vec<bool> = if self.gun_body.is_some() {
            let on_gun = |i: usize| self.gun.get(i).copied().unwrap_or(false);
            self.turret
                .iter()
                .enumerate()
                .map(|(i, &t)| t && !on_gun(i))
                .collect()
        } else {
            self.turret.clone()
        };
        self.turret_shapes = turret_shapes(model, base, &turret).into();
        self.turret_mass = turret_mass(base, &turret);
        if self.turret_mass.is_none() {
            self.gun_body = None;
        }
        self.wreck = wreck_body(model, base);
        // Every vehicle collides by its chassis's shapes alone, as the 360
        // code builds it (0x8216BC08): its hull set when it has one, else
        // all of them (a trailer's wheels' shapes too). No record field
        // changes that.
        let _ = hull;
        self.path_unseen = shapes
            .iter()
            .enumerate()
            .filter(|(_, (_, seen))| !seen)
            .map(|(i, _)| i)
            .collect();
        self.shapes = shapes.into_iter().map(|(v, _)| v).collect();
        self
    }

    /// Whether node `node` spins with its wheel (not just steers with it,
    /// or stays put).
    pub fn spins(&self, node: usize) -> bool {
        self.node_wheel.get(node).copied().flatten().is_some()
            && self.node_rolls.get(node).copied().unwrap_or(true)
    }

    /// Whether it rides on two wheels in line (a motorbike): it is held
    /// upright and leans into its turns.
    pub fn is_bike(&self) -> bool {
        self.wheels.len() == 2
    }

    /// Whether it is a helicopter: it flies and has no wheels.
    pub fn is_helicopter(&self) -> bool {
        self.kind == HELICOPTER
    }

    /// Tilts its tilting parts by `angle` radians about their pivots (do
    /// it after [`spin_rotors`](Self::spin_rotors), so a rotor spins
    /// about its own tilted axis).
    pub fn tilt_parts(&self, nodes: &mut [Mat4], angle: f32) {
        for t in &self.tilts {
            let turn = Mat4::from_translation(t.pivot)
                * Mat4::from_axis_angle(t.axis, angle)
                * Mat4::from_translation(-t.pivot);
            for (m, &on) in nodes.iter_mut().zip(&t.nodes) {
                if on {
                    *m = turn * *m;
                }
            }
        }
    }

    /// Spins its rotors' nodes in `nodes` (as [`posed_nodes`] gives them)
    /// `angle` radians about their hubs.
    ///
    /// [`posed_nodes`]: Self::posed_nodes
    pub fn spin_rotors(&self, nodes: &mut [Mat4], angle: f32) {
        for r in &self.rotors {
            let turn = Mat4::from_translation(r.centre)
                * Mat4::from_axis_angle(r.axis, angle)
                * Mat4::from_translation(-r.centre);
            for (m, &on) in nodes.iter_mut().zip(&r.nodes) {
                if on {
                    *m = turn * *m;
                }
            }
        }
    }

    /// Swings doors in `nodes` (as [`posed_nodes`] gives them): each
    /// `(node name hash, axis, angle)` turns its node `angle` radians about
    /// `axis` in the node's own frame ([`crate::vehicle_door::swung`]), and
    /// every node under it with it.
    ///
    /// [`posed_nodes`]: Self::posed_nodes
    pub fn swing_doors(&self, nodes: &mut [Mat4], doors: &[(u32, Vec3, f32)]) {
        for &(bone, axis, angle) in doors {
            let Some(i) = self.node_names.iter().position(|&n| n == bone) else {
                continue;
            };
            let Some(&at) = nodes.get(i) else { continue };
            let turn = at * Mat4::from_axis_angle(axis, angle) * at.inverse();
            for (j, m) in nodes.iter_mut().enumerate() {
                let mut up = Some(j as u16);
                while let Some(n) = up {
                    if usize::from(n) == i {
                        *m = turn * *m;
                        break;
                    }
                    up = self.node_parents.get(usize::from(n)).copied().flatten();
                }
            }
        }
    }

    /// Whether it is a boat: it floats ([`Physics::set_water`]) and has
    /// no wheels.
    ///
    /// [`Physics::set_water`]: crate::Physics::set_water
    pub fn is_boat(&self) -> bool {
        self.kind == BOAT
    }

    /// Whether its turret's gun fires shells: a tank's, or one its
    /// template fires slower than [`CANNON_RATE`] (the Stryker's), not a
    /// machine gun.
    pub fn cannon(&self) -> bool {
        self.muzzle.is_some()
            && (self.kind == "tank"
                || self
                    .tuning
                    .gun
                    .is_some_and(|g| g.rate_of_fire < CANNON_RATE))
    }

    /// Where the turret turns about, model space: its pivot, or its
    /// mount (its first node's origin).
    pub fn turret_mount(&self) -> Option<Vec3> {
        self.pivot.or_else(|| {
            self.turret
                .iter()
                .zip(&self.rest)
                .find(|(on, _)| **on)
                .map(|(_, m)| m.w_axis.truncate())
        })
    }

    /// How the turret's nodes move with the turret turned `turret`
    /// radians (anticlockwise from above): about the pivot, or its mount.
    pub fn turret_transform(&self, turret: f32) -> Mat4 {
        self.turret_mount().map_or(Mat4::IDENTITY, |at| {
            Mat4::from_translation(at) * Mat4::from_rotation_y(turret) * Mat4::from_translation(-at)
        })
    }

    /// How the gun's nodes tilt with it raised `gun` radians (down is
    /// negative), with the turret straight ahead: about its pivot.
    pub fn gun_tilt(&self, gun: f32) -> Mat4 {
        self.gun_pivot.map_or(Mat4::IDENTITY, |at| {
            // Models face +z: a turn about x by -gun lifts the muzzle.
            Mat4::from_translation(at) * Mat4::from_rotation_x(-gun) * Mat4::from_translation(-at)
        })
    }

    /// Where the muzzle is, and which way the gun points, model space,
    /// with the turret turned `turret` and the gun raised `gun` radians.
    pub fn muzzle_pose(&self, turret: f32, gun: f32) -> Option<(Vec3, Vec3)> {
        let muzzle = self.muzzle?;
        let tilt = if self.muzzle_on_gun {
            self.gun_tilt(gun)
        } else {
            Mat4::IDENTITY
        };
        let m = self.turret_transform(turret) * tilt;
        Some((
            m.transform_point3(muzzle),
            m.transform_vector3(Vec3::Z).normalize_or(Vec3::Z),
        ))
    }

    /// Height of the model's origin above the ground when its wheels rest
    /// on it.
    pub fn ride_height(&self) -> f32 {
        self.wheels
            .iter()
            .map(|w| w.radius - w.centre.y)
            .fold(f32::MIN, f32::max)
            .max(-self.bounds.0.y)
    }

    /// Where the vehicle rests facing `yaw` (radians, as
    /// [`Quat::from_rotation_y`] turns it) with its origin over `at`: on
    /// the plane that best fits the ground under its wheels (its hull's
    /// corners, without three wheels to stand on), tilted to it, with the
    /// lowest wheel touching. `ground` gives the ground's height under a
    /// point; `None` (nothing there, or not loaded yet) under a wheel
    /// gives `None`.
    pub fn rest_on(
        &self,
        at: Vec3,
        yaw: f32,
        ground: impl Fn(Vec3) -> Option<f32>,
    ) -> Option<Rest> {
        let turn = Quat::from_rotation_y(yaw);
        let (lo, hi) = self.bounds;
        let on_ground = |p: Vec3| ground(at + turn * Vec3::new(p.x, 0.0, p.z));
        // Where it touches: the wheels' bottoms, else the hull's corners.
        let wheels: Vec<Vec3> = self
            .wheels
            .iter()
            .map(|w| w.centre - Vec3::Y * w.radius)
            .collect();
        let corners = [(lo.x, lo.z), (hi.x, lo.z), (lo.x, hi.z), (hi.x, hi.z)]
            .map(|(x, z)| Vec3::new(x, lo.y, z));
        let feet = |points: &[Vec3]| -> Option<Vec<(Vec3, f32)>> {
            points.iter().map(|&p| Some((p, on_ground(p)?))).collect()
        };
        let under_wheels = feet(&wheels)?;
        let (touch, (a, b, c)) = match fit_plane(&under_wheels) {
            Some(plane) => (under_wheels, plane),
            None => {
                let touch = feet(&corners)?;
                let plane = fit_plane(&touch)?;
                (touch, plane)
            }
        };
        // The plane is h = a + b x + c z across the footprint.
        let normal = Vec3::new(-b, 1.0, -c).normalize();
        let rotation = turn * Quat::from_rotation_arc(Vec3::Y, normal);
        let y = touch
            .iter()
            .map(|&(p, h)| h - (rotation * p).y)
            .fold(f32::MIN, f32::max);
        // The ground under the footprint, on a grid, against the plane.
        let mut bumps = 0.0f32;
        for i in 0..=2 {
            for j in 0..=4 {
                let x = lo.x + (hi.x - lo.x) * i as f32 / 2.0;
                let z = lo.z + (hi.z - lo.z) * j as f32 / 4.0;
                if let Some(h) = on_ground(Vec3::new(x, 0.0, z)) {
                    bumps = bumps.max(h - (a + b * x + c * z));
                }
            }
        }
        Some(Rest {
            position: Vec3::new(at.x, y, at.z),
            rotation,
            slope: normal.angle_between(Vec3::Y),
            bumps,
        })
    }

    /// Node matrices (model space) with each wheel steered by `steer` and
    /// spun by `spin` radians about its centre (a tracked vehicle's road
    /// wheels rolled `rolled` metres instead), the turret turned `turret`
    /// radians (anticlockwise from above) about its mount, and its gun
    /// raised `gun` radians.
    pub fn posed_nodes(
        &self,
        wheels: &[(f32, f32)],
        rolled: f32,
        turret: f32,
        gun: f32,
    ) -> Vec<Mat4> {
        self.posed_nodes_aimed(wheels, rolled, turret, gun, &[])
    }

    /// [`posed_nodes`](Self::posed_nodes) with each of its
    /// [`aimers`](Self::aimers) turned and tilted as `aims` (yaw, pitch;
    /// one missing stays straight ahead).
    pub fn posed_nodes_aimed(
        &self,
        wheels: &[(f32, f32)],
        rolled: f32,
        turret: f32,
        gun: f32,
        aims: &[(f32, f32)],
    ) -> Vec<Mat4> {
        let about = |centre: Vec3, steer: f32, spin: f32| {
            Mat4::from_translation(centre)
                * Mat4::from_rotation_y(steer)
                * Mat4::from_rotation_x(spin)
                * Mat4::from_translation(-centre)
        };
        // Each wheel spun and steered (its round parts), and only steered
        // (the parts beside it: see `node_rolls`).
        let turns: Vec<(Mat4, Mat4)> = if self.tracked {
            self.road_wheels
                .iter()
                .map(|&(centre, radius)| (about(centre, 0.0, rolled / radius), Mat4::IDENTITY))
                .collect()
        } else {
            self.wheels
                .iter()
                .zip(wheels)
                .map(|(w, &(steer, spin))| {
                    let steer = if w.steers { steer } else { 0.0 };
                    (about(w.centre, steer, spin), about(w.centre, steer, 0.0))
                })
                .collect()
        };
        let aim = self.turret_transform(turret);
        let tilt = self.gun_tilt(gun);
        self.rest
            .iter()
            .zip(&self.node_wheel)
            .enumerate()
            .map(|(i, (&m, w))| {
                let m = match w.and_then(|i| turns.get(i)) {
                    Some(&(spun, steered)) => {
                        if self.node_rolls.get(i).copied().unwrap_or(true) {
                            spun * m
                        } else {
                            steered * m
                        }
                    }
                    None => m,
                };
                let m = self
                    .aimers
                    .iter()
                    .zip(aims)
                    .fold(m, |m, (a, &(yaw, pitch))| a.transform(i, yaw, pitch) * m);
                let m = if self.gun.get(i).copied().unwrap_or(false) {
                    tilt * m
                } else {
                    m
                };
                if self.turret.get(i).copied().unwrap_or(false) {
                    aim * m
                } else {
                    m
                }
            })
            .collect()
    }

    /// Moves each wheel's nodes (from [`posed_nodes`](Self::posed_nodes))
    /// up its spring by `lift` metres, model space, in wheel order
    /// (`Physics::vehicle_wheel_lift`), so its tyre stays on the ground
    /// as the body rides up and down on its springs. A tracked vehicle's
    /// road wheels stay put: its made-up wheels aren't drawn.
    pub fn lift_wheels(&self, nodes: &mut [Mat4], lift: &[f32]) {
        if self.tracked {
            return;
        }
        for (n, w) in nodes.iter_mut().zip(&self.node_wheel) {
            if let Some(&up) = w.and_then(|i| lift.get(i)) {
                *n = Mat4::from_translation(Vec3::Y * up) * *n;
            }
        }
    }
}

/// The plane `h = a + b x + c z` closest (least squares) to `points`'
/// heights over their x and z; `None` unless there are three or more of
/// them spanning an area (none at all: a boat, which has no wheels).
fn fit_plane(points: &[(Vec3, f32)]) -> Option<(f32, f32, f32)> {
    if points.len() < 3 {
        return None;
    }
    let mut m = Mat3::ZERO;
    let mut r = Vec3::ZERO;
    for &(p, h) in points {
        let v = Vec3::new(1.0, p.x, p.z);
        m += Mat3::from_cols(v * v.x, v * v.y, v * v.z);
        r += v * h;
    }
    // About the product of the spreads in x and z (variances, m^2).
    let n = points.len() as f32;
    let spread = m.determinant();
    if spread.is_nan() || spread < 1e-3 * n * n * n {
        return None;
    }
    let s = m.inverse() * r;
    s.is_finite().then_some((s.x, s.y, s.z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::name_hash;

    // An emplaced gun's tripod (the VZ MG's model df3b2fd1): body, the
    // yaw bone 0.5 m up, the pitch bone 0.75 m over that, the weapon's
    // hardpoint 0.3 m ahead of it, and the seat link behind the body.
    // Turned, the hardpoint swings about the yaw bone and the seat stays;
    // tilted up, the hardpoint rises about the pitch bone.
    #[test]
    fn an_emplaced_guns_turret_turns_its_mount_not_its_seat() {
        use crate::anim::{Local, Skeleton};
        let at = |x: f32, y: f32, z: f32| Local {
            translation: Vec3::new(x, y, z),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let skeleton = Skeleton {
            name_hashes: vec![1, 0xc9f1_61f3, 0x5e99_f424, 0x59db_5579, 0x1b81_94a7],
            parents: vec![None, Some(0), Some(1), Some(2), Some(0)],
            rest: vec![
                at(0.0, 0.0, 0.0),
                at(0.0, 0.5, 0.0),
                at(0.0, 0.75, 0.0),
                at(0.0, 0.0, 0.3),
                at(0.0, 0.0, -0.85),
            ],
            inverse_bind: vec![Mat4::IDENTITY; 5],
        };
        let record = crate::seats::TurretRecord {
            name: 0x2afe_1fa8,
            bone_yaw: Some(0xc9f1_61f3),
            bone_pitch: Some(0x5e99_f424),
            yaw: (-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2),
            yaw_speed: 2.0943952,
            pitch: (-0.34906584, std::f32::consts::FRAC_PI_6),
            pitch_speed: 2.0943952,
        };
        let aimer = Aimer::of_skeleton(record, &skeleton).unwrap();
        assert_eq!(aimer.yaw_nodes, [false, true, true, true, false]);
        assert_eq!(aimer.pitch_nodes, [false, false, true, true, false]);
        let rest = skeleton.model_matrices(&skeleton.rest_pose());
        let place = |nodes: &[Mat4], i: usize| nodes[i].w_axis.truncate();
        // A quarter turn: the hardpoint from ahead (+z) to the side (+x).
        let turned = aimer.pose(&rest, std::f32::consts::FRAC_PI_2, 0.0);
        assert!((place(&turned, 3) - Vec3::new(0.3, 1.25, 0.0)).length() < 1e-5);
        assert_eq!(place(&turned, 4), Vec3::new(0.0, 0.0, -0.85));
        // Tilted up 30 degrees: the hardpoint rises in front of its bone.
        let up = aimer.pose(&rest, 0.0, std::f32::consts::FRAC_PI_6);
        let p = place(&up, 3);
        assert!(
            (p - Vec3::new(0.0, 1.25 + 0.15, 0.3 * 0.866_025_4)).length() < 1e-4,
            "{p}"
        );
        // Stepped at its speeds toward a want past its limits.
        let (yaw, pitch) = aimer.step((0.0, 0.0), (3.0, 1.0), 0.5);
        assert!((yaw - 2.0943952 * 0.5).abs() < 1e-5);
        assert!((pitch - std::f32::consts::FRAC_PI_6).abs() < 1e-5);
        // A model without either bone has none.
        let bare = crate::seats::TurretRecord {
            bone_yaw: Some(7),
            bone_pitch: None,
            ..record
        };
        assert!(Aimer::of_skeleton(bare, &skeleton).is_none());
    }

    #[test]
    fn a_tank_stands_on_its_tracks_not_under_its_gun() {
        // The AMX-30's road wheels: five a side; its gun reaches 3 m past
        // the bow, which the model's bounds include and its hull doesn't.
        let road: Vec<(Vec3, f32)> = [2.233, 1.198, 0.164, -0.870, -1.905]
            .iter()
            .flat_map(|&z| {
                [
                    (Vec3::new(-1.371, 0.423, z), 0.352),
                    (Vec3::new(1.371, 0.423, z), 0.352),
                ]
            })
            .collect();
        let hull = (Vec3::new(-1.8, 0.0, -3.3), Vec3::new(1.8, 1.5, 3.3));
        let w = stand_ins(&road, hull);
        assert_eq!(w.len(), 4);
        for (k, (x, z, steers)) in [
            (-1.371, 2.233, true),
            (1.371, 2.233, true),
            (-1.371, -1.905, false),
            (1.371, -1.905, false),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                (w[k].centre.x - x).abs() < 1e-4 && (w[k].centre.z - z).abs() < 1e-4,
                "{:?}",
                w[k]
            );
            assert!((w[k].centre.y - (w[k].radius)).abs() < 1e-4, "{:?}", w[k]);
            assert_eq!(w[k].steers, steers);
        }
        // Without road wheels: under the hull's corners.
        let w = stand_ins(&[], hull);
        assert!(
            w.iter()
                .all(|w| w.centre.z.abs() < 3.3 && w.centre.x.abs() < 1.8)
        );
    }

    #[test]
    fn a_template_sets_the_mass_and_how_far_the_gun_tilts() {
        use crate::tuning::TurretTuning;
        let t = VehicleTuning {
            mass: Some(32_000.0),
            turret: Some(TurretTuning {
                main: true,
                yaw_speed: 1.0,
                yaw_accel: 20.0,
                pitch: (-0.14, 0.44),
                pitch_speed: 1.0,
                pitch_accel: 20.0,
                yaw: None,
            }),
            ..VehicleTuning::default()
        };
        let s = spec().tuned(Some(t));
        assert_eq!(s.mass, 32_000.0);
        assert_eq!(s.elevation, (-0.14, 0.44));
        assert_eq!(spec().tuned(None), spec());
    }

    #[test]
    fn the_driver_sits_and_gets_out_where_the_seat_data_says() {
        use crate::seats::DriverPoints;
        // The Valiant's nodes (round PR3): its fixed markers, the front
        // right seat and door, at -x; the data's driver seat and door at +x.
        let at = |x: f32| Mat4::from_translation(Vec3::new(x, 0.0, 0.4));
        let mut s = spec();
        s.node_names = vec![0xd6eb_e292, 0x688c_4858, 0xd721_fc8c, 0x8869_e876];
        s.rest = vec![at(-0.42), at(-1.7), at(0.42), at(1.7)];
        s.seat = Some(Vec3::new(-0.42, 0.0, 0.4));
        s.door = Some(Vec3::new(-1.7, 0.0, 0.4));
        let data = DriverPoints {
            seat: 0xd721_fc8c,
            door: Some(0x8869_e876),
        };
        let d = s.clone().with_driver_points(Some(data));
        assert_eq!(d.seat, Some(Vec3::new(0.42, 0.0, 0.4)));
        assert_eq!(d.door, Some(Vec3::new(1.7, 0.0, 0.4)));
        // No seat data, or a node the model lacks: the fixed markers stay.
        assert_eq!(s.clone().with_driver_points(None), s);
        let missing = DriverPoints {
            seat: 7,
            door: Some(8),
        };
        assert_eq!(s.clone().with_driver_points(Some(missing)), s);
    }

    fn spec() -> VehicleSpec {
        let wheel = |x: f32, z: f32| WheelSpec {
            centre: Vec3::new(x, 0.35, z),
            radius: 0.35,
            width: 0.25,
            steers: z > 0.0,
        };
        VehicleSpec {
            model: 1,
            kind: "car".into(),
            label: "test".into(),
            node_names: Vec::new(),
            armed: Vec::new(),
            wheels: vec![
                wheel(-0.9, 1.4),
                wheel(0.9, 1.4),
                wheel(-0.9, -1.3),
                wheel(0.9, -1.3),
            ],
            bounds: (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.5, 2.2)),
            seat: None,
            door: None,
            seating: None,
            entrances: Vec::new(),
            tracked: false,
            road_wheels: Vec::new(),
            rest: vec![
                Mat4::IDENTITY,
                Mat4::from_translation(Vec3::new(-0.9, 0.35, 1.4)),
            ],
            node_wheel: vec![None, Some(0)],
            node_rolls: Vec::new(),
            mass: 1400.0,
            turret: vec![false, false],
            pivot: None,
            muzzle: None,
            gun: vec![false, false],
            gun_pivot: None,
            muzzle_on_gun: false,
            elevation: (0.0, 0.0),
            hull: (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.5, 2.2)),
            turret_box: None,
            rotors: Vec::new(),
            tilts: Vec::new(),
            bomb_bay: None,
            node_parents: Vec::new(),
            aimers: Vec::new(),
            damage: None,
            chassis: None,
            shapes: Default::default(),
            material: None,
            path_unseen: Default::default(),
            turret_shapes: Default::default(),
            turret_mass: None,
            gun_body: None,
            wreck: None,
            tuning: VehicleTuning::default(),
        }
    }

    /// An M151 (MG) as its data has it (round seat-turrets): the gunner's
    /// `ControllerTurret` names turret edac857b (yaw -70..70, pitch
    /// -30..30, 100 degrees a second), whose yaw bone c9f161f3 holds the
    /// pitch bone 5e99f424, which holds the gun's hardpoint. The driver
    /// aims nothing. Nodes: 0 root, 1 the wheel, 2 the pintle (yaw), 3 the
    /// gun (pitch), 4 its hardpoint, 0.8 m ahead of the gun's trunnion.
    fn m151() -> VehicleSpec {
        use crate::seats::{Seat, TurretRecord, VehicleSeats};
        let mut s = spec();
        s.node_names = vec![0, 1, 0xc9f1_61f3, 0x5e99_f424, 7];
        s.rest.extend([
            Mat4::from_translation(Vec3::new(0.0, 1.5, -1.0)),
            Mat4::from_translation(Vec3::new(0.0, 1.8, -1.0)),
            Mat4::from_translation(Vec3::new(0.0, 1.8, -0.2)),
        ]);
        s.node_wheel.extend([None, None, None]);
        s.turret = vec![false; 5];
        s.gun = vec![false; 5];
        s.node_parents = vec![None, Some(0), Some(0), Some(2), Some(3)];
        let seat = |name: &str, driver: bool, turret: Option<u32>| Seat {
            name: name_hash(name),
            hardpoint: None,
            driver,
            gunner: !driver,
            hijackable: true,
            player_use: true,
            toss_speed: -1.0,
            min_ditch_speed: 5.0,
            rows: Vec::new(),
            links: Vec::new(),
            turret,
            flags: Default::default(),
        };
        let r = |d: f32| d.to_radians();
        let seating = VehicleSeats {
            seats: vec![
                seat("seat_driver", true, None),
                seat("seat_gunner", false, Some(0xedac_857b)),
            ],
            entrances: Vec::new(),
            turrets: vec![TurretRecord {
                name: 0xedac_857b,
                bone_yaw: Some(0xc9f1_61f3),
                bone_pitch: Some(0x5e99_f424),
                yaw: (r(-70.0), r(70.0)),
                yaw_speed: r(100.0),
                pitch: (r(-30.0), r(30.0)),
                pitch_speed: r(100.0),
            }],
            doors: Vec::new(),
        };
        s.with_seating(Some(std::sync::Arc::new(seating)))
    }

    #[test]
    fn a_gunner_seat_turns_its_own_turret_and_the_driver_none() {
        let s = m151();
        assert_eq!(s.aimers.len(), 1);
        let a = &s.aimers[0];
        // The pintle turns itself, the gun and its hardpoint; the gun tilts
        // with its hardpoint; the body and wheel stay.
        assert_eq!(a.yaw_nodes, [false, false, true, true, true]);
        assert_eq!(a.pitch_nodes, [false, false, false, true, true]);
        assert_eq!(s.aimer_of_seat(name_hash("seat_gunner")), Some(0));
        assert_eq!(s.aimer_of_seat(name_hash("seat_driver")), None);
        // No record turns the main turret's node: the driver's seat turns
        // it (the old rule), the gunner's does not.
        assert!(s.turns_main_turret(Some(name_hash("seat_driver"))));
        assert!(!s.turns_main_turret(Some(name_hash("seat_gunner"))));
        // Turned a quarter left, the hardpoint 0.8 m ahead of the pivot
        // swings to its left (+x), and the pose agrees with node_place.
        let quarter = std::f32::consts::FRAC_PI_2;
        let posed = s.posed_nodes_aimed(&[(0.0, 0.0); 4], 0.0, 0.0, 0.0, &[(quarter, 0.0)]);
        let tip = posed[4].w_axis.truncate();
        assert!(tip.abs_diff_eq(Vec3::new(0.8, 1.8, -1.0), 1e-4), "{tip}");
        let placed = s.node_place(4, 0.0, &[(quarter, 0.0)]).unwrap();
        assert!(placed.abs_diff_eq(posed[4], 1e-5));
        // Tilted up, the tip rises.
        let up = s.posed_nodes_aimed(&[(0.0, 0.0); 4], 0.0, 0.0, 0.0, &[(0.0, 0.5)]);
        assert!(up[4].w_axis.y > 1.8 + 0.3);
        assert!(
            up[2].abs_diff_eq(s.rest[2], 1e-6),
            "the pintle doesn't tilt"
        );
    }

    #[test]
    fn aimed_at_a_point_the_gun_points_at_it() {
        let s = m151();
        let body = Mat4::from_translation(Vec3::new(10.0, 0.0, 20.0));
        // Up and to the left, within the pintle's arc.
        let goal = Vec3::new(10.0 + 6.0, 1.8 + 3.0, 20.0 - 1.0 + 8.0);
        let aim = s.aim_toward(0, body, 0.0, goal);
        let barrel = s.barrel_of(4, 0.0, &[aim]).unwrap();
        let tip = body.transform_point3(s.node_place(4, 0.0, &[aim]).unwrap().w_axis.truncate());
        let pivot = body.transform_point3(Vec3::new(0.0, 1.8, -1.0));
        assert!(barrel.angle_between(goal - pivot) < 1e-4);
        assert!((tip - pivot).angle_between(goal - pivot) < 1e-4);
        assert_eq!(s.barrel_of(1, 0.0, &[aim]), None, "the wheel is no gun");
    }

    #[test]
    fn an_aimer_keeps_to_its_arc_and_speed_and_a_ring_takes_the_short_way() {
        let s = m151();
        let a = &s.aimers[0];
        let r = |d: f32| d.to_radians();
        // A second at 100 degrees a second gets no further than 70 left.
        let (yaw, pitch) = a.step((0.0, 0.0), (r(120.0), r(-80.0)), 1.0);
        assert!((yaw - r(70.0)).abs() < 1e-5 && (pitch - r(-30.0)).abs() < 1e-5);
        // A tenth of a second turns 10 degrees.
        let (yaw, _) = a.step((0.0, 0.0), (r(60.0), 0.0), 0.1);
        assert!((yaw - r(10.0)).abs() < 1e-5);
        // All the way round: from 170 to -170 is 20 degrees through 180.
        let mut ring = a.clone();
        ring.record.yaw = (r(-180.0), r(180.0));
        let (yaw, _) = ring.step((r(170.0), 0.0), (r(-170.0), 0.0), 1.0);
        assert!((yaw - r(190.0)).abs() < 1e-4, "{}", yaw.to_degrees());
    }

    #[test]
    fn the_main_turrets_record_is_left_to_the_main_turret() {
        use crate::seats::VehicleSeats;
        let mut s = m151();
        let mut main = s.seating.as_ref().unwrap().turrets[0];
        main.name = name_hash("main_turret");
        main.bone_yaw = Some(TURRET);
        let mut seating = VehicleSeats::clone(s.seating.as_ref().unwrap());
        seating.turrets.push(main);
        seating.seats[1].turret = Some(main.name);
        s = s.with_seating(Some(std::sync::Arc::new(seating)));
        assert_eq!(s.aimers.len(), 1, "only the pintle's");
        assert!(s.turns_main_turret(Some(name_hash("seat_gunner"))));
        assert!(!s.turns_main_turret(Some(name_hash("seat_driver"))));
    }

    #[test]
    fn a_fender_steers_with_its_wheel_but_does_not_spin() {
        // Node 1 is the front-left tyre; node 2 a fender over it, turning
        // with the same wheel but not round about its axle.
        let mut s = spec();
        let centre = Vec3::new(-0.9, 0.35, 1.4);
        let fender = centre + Vec3::new(0.0, 0.4, 0.1);
        s.rest.push(Mat4::from_translation(fender));
        s.node_wheel.push(Some(0));
        s.node_rolls = vec![true, true, false];
        s.turret.push(false);
        s.gun.push(false);
        assert!(s.spins(1) && !s.spins(2) && !s.spins(0));
        let (steer, spin) = (0.3, 1.0);
        let mut wheels = [(0.0, spin); 4];
        wheels[0].0 = steer;
        let posed = s.posed_nodes(&wheels, 0.0, 0.0, 0.0);
        let turned = |m: Mat4| m.to_scale_rotation_translation().1;
        let both = Quat::from_rotation_y(steer) * Quat::from_rotation_x(spin);
        assert!(turned(posed[1]).angle_between(both) < 1e-4);
        assert!(
            turned(posed[2]).angle_between(Quat::from_rotation_y(steer)) < 1e-4,
            "the fender only steers"
        );
        // Steered about the wheel's centre: it stays above the tyre.
        let at = posed[2].w_axis.truncate();
        let want = centre + Quat::from_rotation_y(steer) * (fender - centre);
        assert!(at.abs_diff_eq(want, 1e-5), "{at}");
        // A rear wheel's fender stays put.
        s.node_wheel[2] = Some(2);
        let posed = s.posed_nodes(&wheels, 0.0, 0.0, 0.0);
        assert!(posed[2].abs_diff_eq(s.rest[2], 1e-5));
    }

    #[test]
    fn turrets_turn_about_their_mount() {
        let mut s = spec();
        let mount = Vec3::new(0.0, 1.5, -0.5);
        s.rest.push(Mat4::from_translation(mount));
        s.rest.push(Mat4::from_translation(mount + Vec3::Z * 2.0));
        s.node_wheel.extend([None, None]);
        s.turret = vec![false, false, true, true];
        let still = [(0.0, 0.0); 4];
        let posed = s.posed_nodes(&still, 0.0, std::f32::consts::FRAC_PI_2, 0.0);
        assert!(posed[2].w_axis.truncate().abs_diff_eq(mount, 1e-5));
        let barrel = posed[3].w_axis.truncate();
        assert!(barrel.abs_diff_eq(mount + Vec3::X * 2.0, 1e-5), "{barrel}");
        assert_eq!(posed[0], s.rest[0], "the hull stays");
    }

    #[test]
    fn the_gun_muzzle_turns_with_the_turret() {
        let mut s = spec();
        let mount = Vec3::new(0.0, 1.5, -0.5);
        s.rest.push(Mat4::from_translation(mount));
        s.node_wheel.push(None);
        s.turret = vec![false, false, true];
        s.muzzle = Some(mount + Vec3::new(0.0, 0.3, 3.0));
        let quarter = s.turret_transform(std::f32::consts::FRAC_PI_2);
        let at = quarter.transform_point3(s.muzzle.unwrap());
        assert!(
            at.abs_diff_eq(mount + Vec3::new(3.0, 0.3, 0.0), 1e-5),
            "{at}"
        );
        // Turned about the pivot instead when there is one.
        s.pivot = Some(Vec3::new(0.0, 1.5, 0.5));
        let at = s
            .turret_transform(std::f32::consts::PI)
            .transform_point3(Vec3::new(0.0, 1.8, 2.5));
        assert!(at.abs_diff_eq(Vec3::new(0.0, 1.8, -1.5), 1e-5), "{at}");
    }

    #[test]
    fn the_gun_tilts_about_its_trunnions_and_turns_with_the_turret() {
        let mut s = spec();
        let mount = Vec3::new(0.0, 1.5, -0.5);
        let trunnion = Vec3::new(0.0, 2.0, 0.5);
        s.rest.push(Mat4::from_translation(mount));
        s.rest.push(Mat4::from_translation(trunnion));
        s.node_wheel.extend([None, None]);
        s.turret = vec![false, false, true, true];
        s.gun = vec![false, false, false, true];
        s.gun_pivot = Some(trunnion);
        s.muzzle = Some(trunnion + Vec3::Z * 4.0);
        s.muzzle_on_gun = true;
        let up = 20f32.to_radians();
        // Straight ahead, raised: the muzzle rises, the turret stays.
        let (at, dir) = s.muzzle_pose(0.0, up).unwrap();
        let want = trunnion + Vec3::new(0.0, up.sin(), up.cos()) * 4.0;
        assert!(at.abs_diff_eq(want, 1e-4), "{at} vs {want}");
        assert!(dir.abs_diff_eq(Vec3::new(0.0, up.sin(), up.cos()), 1e-4));
        let posed = s.posed_nodes(&[(0.0, 0.0); 4], 0.0, 0.0, up);
        assert!(
            posed[2].abs_diff_eq(s.rest[2], 1e-6),
            "the turret stays level"
        );
        assert!(posed[3].w_axis.truncate().abs_diff_eq(trunnion, 1e-5));
        // Turned a quarter (to +x) and raised: up and out to the side.
        let (at, _) = s.muzzle_pose(std::f32::consts::FRAC_PI_2, up).unwrap();
        assert!(at.y > trunnion.y + 1.0 && at.x > 3.0, "{at}");
    }

    #[test]
    fn wheels_ride_up_and_down_their_springs() {
        let s = spec();
        let mut nodes = s.posed_nodes(&[(0.0, 0.0); 4], 0.0, 0.0, 0.0);
        let before = nodes.clone();
        s.lift_wheels(&mut nodes, &[0.05, -0.1, 0.0, 0.0]);
        for (i, (a, b)) in before.iter().zip(&nodes).enumerate() {
            let up = match s.node_wheel[i] {
                Some(0) => 0.05,
                Some(1) => -0.1,
                _ => 0.0,
            };
            let moved = b.w_axis.truncate() - a.w_axis.truncate();
            assert!(moved.abs_diff_eq(Vec3::Y * up, 1e-6), "{i}: {moved}");
        }
    }

    #[test]
    fn wheels_turn_their_nodes_about_their_centres() {
        let s = spec();
        let posed = s.posed_nodes(
            &[(0.5, 1.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
            0.0,
            0.0,
            0.0,
        );
        assert_eq!(posed[0], Mat4::IDENTITY);
        // The wheel node sits at the wheel's centre, so turning keeps it
        // there.
        let centre = posed[1].transform_point3(Vec3::ZERO);
        assert!((centre - Vec3::new(-0.9, 0.35, 1.4)).length() < 1e-5);
        assert!(!posed[1].abs_diff_eq(s.rest[1], 1e-3));
    }

    #[test]
    fn rides_on_its_wheels() {
        assert!(spec().ride_height().abs() < 1e-6);
    }

    #[test]
    fn rests_tilted_on_a_slope_with_its_wheels_down() {
        let s = spec();
        // Ground rising 20 degrees towards +x, 5 m up at the origin.
        let tan = 20f32.to_radians().tan();
        let ground = |p: Vec3| Some(5.0 + p.x * tan);
        // Facing +x (a quarter turn): nose up the slope.
        let yaw = std::f32::consts::FRAC_PI_2;
        let at = Vec3::new(10.0, 0.0, 3.0);
        let rest = s.rest_on(at, yaw, ground).unwrap();
        assert!((rest.slope - 20f32.to_radians()).abs() < 1e-3, "{rest:?}");
        assert!(rest.bumps < 0.05, "{rest:?}");
        let ahead = rest.rotation * Vec3::Z;
        assert!(ahead.x > 0.9 && ahead.y > 0.3, "nose up the slope: {ahead}");
        // Every wheel's bottom on the ground (within a few cm).
        for w in &s.wheels {
            let bottom = rest.position + rest.rotation * (w.centre - Vec3::Y * w.radius);
            let h = ground(bottom).unwrap();
            assert!((bottom.y - h).abs() < 0.08, "{bottom} vs {h}");
        }
        // Nothing loaded under it: nowhere to rest yet.
        assert!(s.rest_on(at, yaw, |_| None).is_none());
    }

    // Round 88 hopped the start boat onto rocks: with no wheels its rest
    // came out NaN (a plane fitted to no points), and so did she when she
    // got out of it.
    #[test]
    fn a_boat_without_wheels_rests_on_its_hull() {
        let mut s = spec();
        s.kind = BOAT.into();
        s.wheels.clear();
        s.node_wheel = vec![None, None];
        let tan = 10f32.to_radians().tan();
        let ground = |p: Vec3| Some(-1.0 + p.z * tan);
        let rest = s.rest_on(Vec3::new(4.0, 0.0, 2.0), 0.3, ground).unwrap();
        assert!(
            rest.position.is_finite() && rest.rotation.is_finite(),
            "{rest:?}"
        );
        assert!((rest.slope - 10f32.to_radians()).abs() < 0.02, "{rest:?}");
        assert!(s.ride_height().is_finite());
        assert_eq!(fit_plane(&[]), None);
        assert_eq!(fit_plane(&[(Vec3::X, 1.0), (Vec3::Z, 1.0)]), None);
    }

    #[test]
    fn a_ledge_under_the_hull_shows_as_bumps() {
        let s = spec();
        // Flat, but a step up 1 m under the front half's middle.
        let ground = |p: Vec3| {
            Some(if p.z > 0.5 && p.x.abs() < 0.5 {
                1.0
            } else {
                0.0
            })
        };
        let rest = s.rest_on(Vec3::ZERO, 0.0, ground).unwrap();
        assert!(rest.slope < 1e-3, "{rest:?}");
        assert!((rest.bumps - 1.0).abs() < 1e-3, "{rest:?}");
        assert!(rest.position.abs_diff_eq(Vec3::ZERO, 1e-4), "{rest:?}");
    }
}
