//! Drivable vehicles: what the island's vehicle models are, how they are
//! built, and how they drive. [`catalogue`] also finds the person models.
//!
//! [`catalogue`] finds the vehicle models in the archive by their texture
//! names, and [`VehicleSpec`] reads one model's wheels, seat and door with
//! the heuristics in `m2_assets::vehicle`. [`Physics`](crate::Physics)
//! drives a spec as a Rapier rigid body on ray-cast wheels: each wheel is a
//! suspension ray, the front axle steers, every wheel drives.
//!
//! # Provenance
//!
//! Clean-room. The kinds, wheels and markers come from the game's data as
//! `m2_assets::vehicle` describes; the masses, engine forces, suspension
//! and grip are invented arcade tuning, as in the browser viewer.

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

/// How to build and draw one vehicle model.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleSpec {
    pub model: u32,
    pub kind: String,
    pub label: String,
    pub wheels: Vec<WheelSpec>,
    /// Model-space bounds.
    pub bounds: (Vec3, Vec3),
    /// Driver's seat and door markers, model space.
    pub seat: Option<Vec3>,
    pub door: Option<Vec3>,
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
    /// Mass, kg (by kind; invented).
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
            wheels = stand_ins(&road_wheels, hull);
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
            tracked,
            road_wheels,
            rest,
            node_wheel,
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

    /// Whether it rides on two wheels in line (a motorbike): it is held
    /// upright and leans into its turns.
    pub fn is_bike(&self) -> bool {
        self.wheels.len() == 2
    }

    /// Whether it is a helicopter: it flies and has no wheels.
    pub fn is_helicopter(&self) -> bool {
        self.kind == HELICOPTER
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

    /// How the turret's nodes move with the turret turned `turret`
    /// radians (anticlockwise from above): about the pivot, or its mount.
    pub fn turret_transform(&self, turret: f32) -> Mat4 {
        let mount = self.pivot.or_else(|| {
            self.turret
                .iter()
                .zip(&self.rest)
                .find(|(on, _)| **on)
                .map(|(_, m)| m.w_axis.truncate())
        });
        mount.map_or(Mat4::IDENTITY, |at| {
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
        let about = |centre: Vec3, steer: f32, spin: f32| {
            Mat4::from_translation(centre)
                * Mat4::from_rotation_y(steer)
                * Mat4::from_rotation_x(spin)
                * Mat4::from_translation(-centre)
        };
        let turns: Vec<Mat4> = if self.tracked {
            self.road_wheels
                .iter()
                .map(|&(centre, radius)| about(centre, 0.0, rolled / radius))
                .collect()
        } else {
            self.wheels
                .iter()
                .zip(wheels)
                .map(|(w, &(steer, spin))| {
                    about(w.centre, if w.steers { steer } else { 0.0 }, spin)
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
                    Some(&t) => t * m,
                    None => m,
                };
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
            tracked: false,
            road_wheels: Vec::new(),
            rest: vec![
                Mat4::IDENTITY,
                Mat4::from_translation(Vec3::new(-0.9, 0.35, 1.4)),
            ],
            node_wheel: vec![None, Some(0)],
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
            tuning: VehicleTuning::default(),
        }
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
