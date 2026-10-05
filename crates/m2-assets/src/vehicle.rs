//! Vehicles: what kind a model is and where its wheels are.
//!
//! Model nodes are named only by hash, so both come from the data itself:
//!
//! - The kind comes from texture names. Vehicle textures are named
//!   `<faction>_veh_<kind>_<model>...` (`civ_veh_car_...`,
//!   `pmc_veh_truck_monster_...`, `gr_veh_apc_m113_...`), so
//!   [`kind_from_textures`] takes the first such name that is not a shared
//!   texture (wreck, glass, wheel and similar).
//! - Wheels are found by shape ([`wheels_from_boxes`]). A wheel's geometry
//!   is round seen from the side (as tall as it is long), thinner than it
//!   is tall, reaches down to the model's floor, and is centred on a node
//!   whose subtree can be turned to spin or steer it.
//!
//! Both are heuristics over the data, not records the game stores in this
//! form; [`find_wheels`] reports what it found and the caller decides what
//! to do without wheels.
//!
//! # Provenance
//!
//! Format-from-samples: the naming pattern and the wheel layout were read
//! from the vehicle models and textures of a legally installed copy (for
//! example a PMC car whose four wheel nodes sit at x ±0.92, y 0.35, z -1.32
//! and +1.39 with 0.35 m wheels). No bytes from the archives are committed.

use m2_math::glam::{Mat4, Vec3};

use crate::model::{GeometryKind, Model};
use crate::vertex::DeclUsage;

/// One wheel.
#[derive(Debug, Clone, PartialEq)]
pub struct Wheel {
    /// The nodes whose subtrees are the wheel (copies, such as a damaged
    /// one, share the centre): turn them to spin or steer it.
    pub nodes: Vec<u16>,
    /// The node's origin, model space: the wheel's centre.
    pub centre: [f32; 3],
    pub radius: f32,
    pub width: f32,
}

/// A geometry bounding box of one node's own segments, model space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeBox {
    pub node: u16,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Node name hash of the driver's seat marker: a leaf node at floor height
/// at the left front seat, the same hash on every car and truck sampled
/// (PMC car x -0.37, civilian car -0.42, HMMWV -0.73, M35 -0.42, semi
/// -0.42). The right front seat is [`PASSENGER_SEAT`].
pub const DRIVER_SEAT: u32 = 0xd6eb_e292;
/// The right front seat marker.
pub const PASSENGER_SEAT: u32 = 0xd721_fc8c;
/// The marker outside the driver's door, where a character stands to get
/// in (PMC car x -1.40, civilian car -1.70, HMMWV -1.93, semi -2.10).
pub const DRIVER_DOOR: u32 = 0x688c_4858;
/// The marker outside the passenger's door.
pub const PASSENGER_DOOR: u32 = 0x8869_e876;

/// Node name hashes of a helicopter's main rotor: the blades and the blur
/// disc (texture `global_veh_rotorblur`), both centred on the hub. Shared
/// by the UH-1 and the Mi-26 (hub at y 3.55 and 8.16).
pub const MAIN_ROTOR: [u32; 2] = [0x2c9f_4cec, 0x4cc6_28fa];
/// The tail rotor's blades and blur disc, flat in the y-z plane at the end
/// of the tail boom (UH-1 z -7.34, Mi-26 z -19.79).
pub const TAIL_ROTOR: [u32; 2] = [0x05f6_5f7e, 0xc065_7b1c];

/// A rotor: the nodes to turn, the hub, and the axis it turns about
/// (model space: `[0, 1, 0]` for a main rotor, `[1, 0, 0]` for a tail).
#[derive(Debug, Clone, PartialEq)]
pub struct Rotor {
    pub nodes: Vec<u16>,
    pub centre: [f32; 3],
    pub axis: [f32; 3],
}

/// The main and tail rotors of a helicopter model, by their marker names.
pub fn rotors(model: &Model) -> Vec<Rotor> {
    let mut out = Vec::new();
    for (names, axis) in [(MAIN_ROTOR, [0.0, 1.0, 0.0]), (TAIL_ROTOR, [1.0, 0.0, 0.0])] {
        let nodes: Vec<u16> = (0..model.nodes.len())
            .filter(|&i| names.contains(&model.nodes[i].name_hash))
            .map(|i| i as u16)
            .collect();
        let Some(&first) = nodes.first() else {
            continue;
        };
        let Some(m) = model.node_to_model(first) else {
            continue;
        };
        out.push(Rotor {
            nodes,
            centre: m.w_axis.truncate().to_array(),
            axis,
        });
    }
    out
}

/// The model-space position of the first node named `hash`.
pub fn marker(model: &Model, hash: u32) -> Option<[f32; 3]> {
    let i = model.nodes.iter().position(|n| n.name_hash == hash)?;
    Some(model.node_to_model(i as u16)?.w_axis.truncate().to_array())
}

/// Texture-name words that mark textures shared across vehicles.
const SHARED: [&str; 12] = [
    "ruin",
    "wreck",
    "glass",
    "wheel",
    "blur",
    "transparent",
    "frame",
    "rust",
    "grill",
    "weapon",
    "turbosquid",
    "civilian",
];

/// A vehicle's kind and name from its texture names (lower case):
/// `("car", "pmc_veh_car_vanquish")` for `pmc_veh_car_vanquish_dm`.
pub fn kind_from_textures<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Option<(String, String)> {
    // `global_veh_<model>` textures are shared and name a model, not a
    // kind: prefer a faction's own texture.
    let names: Vec<&str> = names.into_iter().collect();
    let (faction, global): (Vec<&str>, Vec<&str>) =
        names.into_iter().partition(|n| !n.starts_with("global_"));
    for name in faction.into_iter().chain(global) {
        let Some(at) = name.find("_veh_") else {
            continue;
        };
        if SHARED.iter().any(|w| name.contains(w)) {
            continue;
        }
        let rest = &name[at + 5..];
        let kind = rest.split('_').next().filter(|k| !k.is_empty())?;
        // The name: up to the kind and one more word, without map suffixes.
        let mut words = rest.split('_');
        let label = match (words.next(), words.next()) {
            (Some(k), Some(m)) if !["dm", "sm", "nm", "lod"].contains(&m) => {
                format!("{}_veh_{k}_{m}", &name[..at])
            }
            (Some(k), _) => format!("{}_veh_{k}", &name[..at]),
            _ => continue,
        };
        return Some((kind.trim().to_owned(), label));
    }
    None
}

/// Finds wheels among per-node geometry boxes. `origins` and `parents`
/// describe every node (model space); `floor` is the model's lowest point.
pub fn wheels_from_boxes(
    origins: &[[f32; 3]],
    parents: &[Option<u16>],
    boxes: &[NodeBox],
    floor: f32,
) -> Vec<Wheel> {
    // Wheel root -> union of its boxes.
    let mut roots: Vec<(u16, [f32; 3], [f32; 3])> = Vec::new();
    for b in boxes {
        let s = [0, 1, 2].map(|a| b.max[a] - b.min[a]);
        let d = s[1].max(s[2]);
        let round = (s[1] - s[2]).abs() <= 0.15 * d;
        let thin = s[0] <= 0.85 * d && s[0] >= 0.03;
        let grounded = b.min[1] <= floor + 0.1 * d + 0.05;
        if d < 0.3 || !round || !thin || !grounded {
            continue;
        }
        let c = [0, 1, 2].map(|a| 0.5 * (b.min[a] + b.max[a]));
        // A wheel turns about a node on its axle: at its middle, or (the
        // Stryker's) at the hub's inner face.
        let at = |n: u16| {
            origins.get(usize::from(n)).is_some_and(|o| {
                let slack = 0.05 * d + 0.02;
                let off_axle = ((o[1] - c[1]).powi(2) + (o[2] - c[2]).powi(2)).sqrt();
                off_axle <= slack && (o[0] - c[0]).abs() <= 0.5 * s[0] + slack
            })
        };
        // The topmost ancestor (or the node itself) still at the centre.
        let mut root = None;
        let mut n = Some(b.node);
        for _ in 0..=origins.len() {
            let Some(cur) = n else { break };
            if at(cur) {
                root = Some(cur);
            } else if root.is_some() {
                break;
            }
            n = parents.get(usize::from(cur)).copied().flatten();
        }
        let Some(root) = root else { continue };
        match roots.iter_mut().find(|r| r.0 == root) {
            Some(r) => {
                for a in 0..3 {
                    r.1[a] = r.1[a].min(b.min[a]);
                    r.2[a] = r.2[a].max(b.max[a]);
                }
            }
            None => roots.push((root, b.min, b.max)),
        }
    }
    let ancestor_of = |a: u16, mut n: u16| {
        for _ in 0..=origins.len() {
            match parents.get(usize::from(n)).copied().flatten() {
                Some(p) if p == a => return true,
                Some(p) => n = p,
                None => return false,
            }
        }
        false
    };
    let mut wheels: Vec<Wheel> = Vec::new();
    for &(node, min, max) in roots
        .iter()
        .filter(|r| !roots.iter().any(|o| o.0 != r.0 && ancestor_of(r.0, o.0)))
    {
        // On its axle, across the tyre's middle.
        let mut centre = origins[usize::from(node)];
        centre[0] = 0.5 * (min[0] + max[0]);
        let radius = 0.5 * (max[1] - min[1]).max(max[2] - min[2]);
        let width = max[0] - min[0];
        // Copies at the same centre are one wheel: on one axle, and
        // overlapping across it (the Honda CRX's tyre and its copy sit
        // 5 cm apart).
        let same = wheels.iter_mut().find(|w| {
            (1..3).all(|a| (w.centre[a] - centre[a]).abs() <= 0.1 * radius.max(w.radius))
                && (w.centre[0] - centre[0]).abs() <= 0.5 * width.max(w.width)
        });
        match same {
            Some(w) => {
                w.nodes.push(node);
                w.radius = w.radius.max(radius);
                w.width = w.width.max(width);
            }
            None => wheels.push(Wheel {
                nodes: vec![node],
                centre,
                radius,
                width,
            }),
        }
    }
    wheels.sort_by(|a, b| {
        b.centre[2]
            .total_cmp(&a.centre[2])
            .then(a.centre[0].total_cmp(&b.centre[0]))
    });
    wheels
}

/// Per-node geometry boxes of a model's finest complete level of detail,
/// model space (skinned geometry is left out: it has no single node).
pub fn node_boxes(model: &Model) -> Vec<NodeBox> {
    let lod = (0..model.info.lod_count)
        .find(|&l| model.has_lod(l))
        .unwrap_or(0);
    let mut out: Vec<NodeBox> = Vec::new();
    for (segment, geom) in model.lod(lod) {
        if geom.kind == GeometryKind::Skin {
            continue;
        }
        let to_model = model.node_to_model(segment.node).unwrap_or(Mat4::IDENTITY);
        for g in &geom.groups {
            let Some(pos) = g
                .decl
                .find(DeclUsage::Position, 0)
                .and_then(|e| g.vertices.read(e))
            else {
                continue;
            };
            for p in pos {
                let v = to_model
                    .transform_point3(Vec3::new(p[0], p[1], p[2]))
                    .to_array();
                if !v.iter().all(|c| c.is_finite()) {
                    continue;
                }
                match out.iter_mut().find(|b| b.node == segment.node) {
                    Some(b) => {
                        b.min = [0, 1, 2].map(|a| b.min[a].min(v[a]));
                        b.max = [0, 1, 2].map(|a| b.max[a].max(v[a]));
                    }
                    None => out.push(NodeBox {
                        node: segment.node,
                        min: v,
                        max: v,
                    }),
                }
            }
        }
    }
    out
}

/// The wheels of a vehicle model (with its streamed chunks added).
pub fn find_wheels(model: &Model) -> Vec<Wheel> {
    let n = model.nodes.len();
    let origins: Vec<[f32; 3]> = (0..n)
        .map(|i| {
            model
                .node_to_model(i as u16)
                .map_or([0.0; 3], |m| m.w_axis.truncate().to_array())
        })
        .collect();
    let parents: Vec<Option<u16>> = model.nodes.iter().map(|nd| nd.parent).collect();
    let boxes = node_boxes(model);
    let floor = boxes.iter().map(|b| b.min[1]).fold(f32::INFINITY, f32::min);
    wheels_from_boxes(&origins, &parents, &boxes, floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_kinds_from_textures() {
        let k = |names: &[&str]| kind_from_textures(names.iter().copied());
        assert_eq!(
            k(&["global_veh_civilianruin", "pmc_veh_car_vanquish_dm"]),
            Some(("car".into(), "pmc_veh_car_vanquish".into()))
        );
        assert_eq!(
            k(&["gr_veh_apc_m113_sm"]),
            Some(("apc".into(), "gr_veh_apc_m113".into()))
        );
        assert_eq!(k(&["vz_veh_truck_m151_wheel_dm", "city_bld_wall"]), None);
        assert_eq!(
            k(&["al_veh_tank_dm"]),
            Some(("tank".into(), "al_veh_tank".into()))
        );
    }

    /// A car: chassis node 0 with a body box, four wheel pivots each with
    /// a child holding the tyre, and a round spare wheel on the roof.
    #[test]
    fn finds_four_grounded_wheels() {
        let mut origins = vec![[0.0, 0.0, 0.0]];
        let mut parents = vec![None];
        let mut boxes = vec![NodeBox {
            node: 0,
            min: [-1.0, 0.25, -2.4],
            max: [1.0, 1.3, 2.4],
        }];
        for (x, z) in [(-0.9, 1.4), (0.9, 1.4), (-0.9, -1.3), (0.9, -1.3)] {
            let pivot = origins.len() as u16;
            origins.push([x, 0.35, z]);
            parents.push(Some(0));
            origins.push([x, 0.35, z]);
            parents.push(Some(pivot));
            boxes.push(NodeBox {
                node: pivot + 1,
                min: [x - 0.12, 0.0, z - 0.35],
                max: [x + 0.12, 0.7, z + 0.35],
            });
        }
        origins.push([0.0, 1.6, 0.0]);
        parents.push(Some(0));
        boxes.push(NodeBox {
            node: 9,
            min: [-0.1, 1.3, -0.3],
            max: [0.1, 1.9, 0.3],
        });
        let w = wheels_from_boxes(&origins, &parents, &boxes, 0.0);
        assert_eq!(w.len(), 4, "{w:?}");
        // Front (+z) first, left to right; the pivots are the roots.
        assert_eq!(w[0].nodes, [1]);
        assert_eq!(w[0].centre, [-0.9, 0.35, 1.4]);
        assert!((w[0].radius - 0.35).abs() < 1e-5 && (w[0].width - 0.24).abs() < 1e-5);
        assert_eq!(w[3].centre, [0.9, 0.35, -1.3]);
    }

    #[test]
    fn finds_wheels_turning_about_the_hubs_inner_face() {
        // The Stryker's: each tyre's node sits on its axle at the inner
        // face, 0.19 m in from the tyre's middle.
        let mut origins = vec![[0.0, 0.0, 0.0]];
        let mut parents = vec![None];
        let mut boxes = vec![NodeBox {
            node: 0,
            min: [-1.5, 0.45, -3.7],
            max: [1.5, 2.5, 3.7],
        }];
        for z in [2.148, 0.916, -0.593, -1.826] {
            for side in [-1.0f32, 1.0] {
                let node = origins.len() as u16;
                origins.push([0.985 * side, 0.578, z]);
                parents.push(Some(0));
                let (a, b) = (0.985 * side, 1.369 * side);
                boxes.push(NodeBox {
                    node,
                    min: [a.min(b), 0.0, z - 0.578],
                    max: [a.max(b), 1.155, z + 0.578],
                });
            }
        }
        let w = wheels_from_boxes(&origins, &parents, &boxes, 0.0);
        assert_eq!(w.len(), 8, "{w:?}");
        assert!((w[0].centre[0] + 1.177).abs() < 1e-3, "{:?}", w[0].centre);
        assert!((w[0].radius - 0.5775).abs() < 1e-3 && (w[0].width - 0.384).abs() < 1e-3);
    }

    #[test]
    fn merges_copies_of_a_wheel() {
        // A pristine and a damaged tyre, each on its own node at one centre.
        let origins = [[0.0, 0.0, 0.0], [0.9, 0.35, 1.4], [0.9, 0.35, 1.4]];
        let parents = [None, Some(0), Some(0)];
        let tyre = |node| NodeBox {
            node,
            min: [0.78, 0.0, 1.05],
            max: [1.02, 0.7, 1.75],
        };
        let w = wheels_from_boxes(&origins, &parents, &[tyre(1), tyre(2)], 0.0);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].nodes, [1, 2]);
    }

    #[test]
    fn merges_copies_of_a_wheel_a_little_apart_across_the_axle() {
        // The Honda CRX's: each tyre twice, 5 cm apart along the axle.
        let origins = [[0.0, 0.0, 0.0], [0.81, 0.29, 1.09], [0.76, 0.29, 1.09]];
        let parents = [None, Some(0), Some(0)];
        let tyre = |node, x: f32| NodeBox {
            node,
            min: [x - 0.1, 0.0, 0.8],
            max: [x + 0.1, 0.58, 1.38],
        };
        let w = wheels_from_boxes(&origins, &parents, &[tyre(1, 0.81), tyre(2, 0.76)], 0.0);
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].nodes, [1, 2]);
        // A truck's twin tyres, a tyre's width apart, are two wheels.
        let origins = [[0.0, 0.0, 0.0], [0.8, 0.5, -2.0], [1.1, 0.5, -2.0]];
        let w = wheels_from_boxes(
            &origins,
            &parents,
            &[
                NodeBox {
                    node: 1,
                    min: [0.66, 0.0, -2.5],
                    max: [0.94, 1.0, -1.5],
                },
                NodeBox {
                    node: 2,
                    min: [0.96, 0.0, -2.5],
                    max: [1.24, 1.0, -1.5],
                },
            ],
            0.0,
        );
        assert_eq!(w.len(), 2, "{w:?}");
    }

    #[test]
    fn ignores_boxes_off_any_node_origin() {
        // A round grounded box whose centre is no node's origin.
        let w = wheels_from_boxes(
            &[[0.0, 0.0, 0.0]],
            &[None],
            &[NodeBox {
                node: 0,
                min: [0.8, 0.0, 1.0],
                max: [1.0, 0.7, 1.7],
            }],
            0.0,
        );
        assert!(w.is_empty());
    }
}
