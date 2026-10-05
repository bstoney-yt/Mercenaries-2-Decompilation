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
    /// Each node's rest transform (node to model).
    pub rest: Vec<Mat4>,
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

/// Node name hash of a gun's muzzle marker (the hand weapons have it).
pub const GUN_MUZZLE: u32 = 0xb148_6ef3;

/// The node a turret turns about, in every vehicle that has one: both the
/// intact and the wrecked turret hang from it.
pub const TURRET_MOUNT: u32 = 0xc851_b695;

/// The turret itself, under its mount, with its origin on the turret
/// ring's middle (the M1A1's, the Stryker's).
pub const TURRET: u32 = 0x7707_65e4;

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
        let usable = (4..=10).contains(&found.len())
            && found.iter().all(|w| (0.15..=1.3).contains(&w.radius));
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
        let wheels: Vec<WheelSpec> = if usable && !tracked {
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
            // Four stand-in wheels under the hull's corners.
            let (lo, hi) = bounds;
            let r = 0.45;
            let (x, z) = ((hi.x - lo.x) * 0.4, (hi.z - lo.z) * 0.35);
            let c = (lo + hi) * 0.5;
            [(-x, z), (x, z), (-x, -z), (x, -z)]
                .iter()
                .map(|&(dx, dz)| WheelSpec {
                    centre: Vec3::new(c.x + dx, lo.y + r, c.z + dz),
                    radius: r,
                    width: 0.5,
                    steers: dz > 0.0,
                })
                .collect()
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
            .filter_map(|(i, _)| model.node_to_model(i as u16))
            .map(|m| m.w_axis.truncate())
            .max_by(|a, b| a.z.total_cmp(&b.z))
            .or_else(|| {
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
        })
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
        let (touch, (a, b, c)) = match fit_plane(&feet(&wheels)?) {
            Some(plane) => (feet(&wheels)?, plane),
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
    /// wheels rolled `rolled` metres instead), and the turret turned
    /// `turret` radians (anticlockwise from above) about its mount.
    pub fn posed_nodes(&self, wheels: &[(f32, f32)], rolled: f32, turret: f32) -> Vec<Mat4> {
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
        self.rest
            .iter()
            .zip(&self.node_wheel)
            .enumerate()
            .map(|(i, (&m, w))| {
                let m = match w.and_then(|i| turns.get(i)) {
                    Some(&t) => t * m,
                    None => m,
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
/// heights over their x and z; `None` unless they span an area.
fn fit_plane(points: &[(Vec3, f32)]) -> Option<(f32, f32, f32)> {
    let mut m = Mat3::ZERO;
    let mut r = Vec3::ZERO;
    for &(p, h) in points {
        let v = Vec3::new(1.0, p.x, p.z);
        m += Mat3::from_cols(v * v.x, v * v.y, v * v.z);
        r += v * h;
    }
    // About the product of the spreads in x and z (variances, m^2).
    let n = points.len() as f32;
    if m.determinant() < 1e-3 * n * n * n {
        return None;
    }
    let s = m.inverse() * r;
    Some((s.x, s.y, s.z))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let posed = s.posed_nodes(&still, 0.0, std::f32::consts::FRAC_PI_2);
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
    fn wheels_turn_their_nodes_about_their_centres() {
        let s = spec();
        let posed = s.posed_nodes(&[(0.5, 1.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)], 0.0, 0.0);
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
