//! Drivable vehicles: what the island's vehicle models are, how they are
//! built, and how they drive.
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
use m2_math::glam::{Mat4, Vec3};

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

/// Every vehicle model in the archive, found by texture name. This reads
/// every block, so run it off the main thread.
pub fn catalogue<R: Read + Seek>(
    source: &mut AssetSource<R>,
) -> Result<Vec<CatalogueEntry>, AssetError> {
    let blocks = source.archive().index().blocks().len();
    let mut texture_names: HashMap<u32, String> = HashMap::new();
    let mut model_textures: Vec<(u32, Vec<u32>)> = Vec::new();
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
                        model_textures.push((
                            e.name_hash,
                            m.materials
                                .iter()
                                .flat_map(|mt| mt.textures.iter().copied())
                                .collect(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    let mut out: Vec<CatalogueEntry> = model_textures
        .into_iter()
        .filter_map(|(model, textures)| {
            let names = textures
                .iter()
                .filter_map(|t| texture_names.get(t).map(String::as_str));
            let (kind, label) = vehicle::kind_from_textures(names)?;
            Some(CatalogueEntry { model, kind, label })
        })
        .collect();
    out.sort_by(|a, b| (&a.kind, &a.label, a.model).cmp(&(&b.kind, &b.label, b.model)));
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
    /// Tracked vehicles (tanks, APCs) drive on wheels found or made up;
    /// their wheels are not turned when drawn.
    pub tracked: bool,
    /// Each node's rest transform (node to model).
    pub rest: Vec<Mat4>,
    /// The wheel each node turns with, if any.
    pub node_wheel: Vec<Option<usize>>,
    /// Mass, kg (by kind; invented).
    pub mass: f32,
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
        let tracked = entry.kind == "tank" || entry.kind == "apc";
        let found = vehicle::find_wheels(model);
        let usable = (4..=10).contains(&found.len())
            && found.iter().all(|w| (0.15..=1.3).contains(&w.radius));
        let n = model.nodes.len();
        let mut node_wheel = vec![None; n];
        let wheels: Vec<WheelSpec> = if usable {
            let front = found
                .iter()
                .map(|w| w.centre[2])
                .fold(f32::NEG_INFINITY, f32::max);
            for (i, w) in found.iter().enumerate() {
                for &root in &w.nodes {
                    // The wheel's subtree: every node with `root` as an
                    // ancestor (or itself).
                    for (k, slot) in node_wheel.iter_mut().enumerate() {
                        let mut at = Some(k as u16);
                        for _ in 0..=n {
                            match at {
                                Some(c) if c == root => {
                                    *slot = Some(i);
                                    break;
                                }
                                Some(c) => {
                                    at = model.nodes.get(usize::from(c)).and_then(|x| x.parent)
                                }
                                None => break,
                            }
                        }
                    }
                }
            }
            found
                .iter()
                .map(|w| WheelSpec {
                    centre: Vec3::from(w.centre),
                    radius: w.radius,
                    width: w.width,
                    steers: w.centre[2] >= front - 0.3,
                })
                .collect()
        } else if tracked {
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
            rest,
            node_wheel,
            mass,
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

    /// Node matrices (model space) with each wheel steered by `steer` and
    /// spun by `spin` radians about its centre.
    pub fn posed_nodes(&self, wheels: &[(f32, f32)]) -> Vec<Mat4> {
        let turns: Vec<Mat4> = self
            .wheels
            .iter()
            .zip(wheels)
            .map(|(w, &(steer, spin))| {
                if self.tracked {
                    return Mat4::IDENTITY;
                }
                Mat4::from_translation(w.centre)
                    * Mat4::from_rotation_y(if w.steers { steer } else { 0.0 })
                    * Mat4::from_rotation_x(spin)
                    * Mat4::from_translation(-w.centre)
            })
            .collect();
        self.rest
            .iter()
            .zip(&self.node_wheel)
            .map(|(&m, w)| match w.and_then(|i| turns.get(i)) {
                Some(&t) => t * m,
                None => m,
            })
            .collect()
    }
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
            rest: vec![
                Mat4::IDENTITY,
                Mat4::from_translation(Vec3::new(-0.9, 0.35, 1.4)),
            ],
            node_wheel: vec![None, Some(0)],
            mass: 1400.0,
        }
    }

    #[test]
    fn wheels_turn_their_nodes_about_their_centres() {
        let s = spec();
        let posed = s.posed_nodes(&[(0.5, 1.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]);
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
}
