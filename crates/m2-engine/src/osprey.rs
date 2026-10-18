//! A body for the cut V-22 Osprey.
//!
//! The game keeps three Osprey templates (`V22 (Delivery)`, `V22 (Driver)`,
//! `V22 - DO NOT USE`), its PDA icon, its seat texture, seven crew clips
//! and its animation-table rows, but its model `global_veh_v22osprey`
//! (`0x2e7cb0b5`) is in none of the PC archives (cut content survey,
//! 2026-10-18). This module stands a model in for it, with two proprotors
//! that turn the way the game's helicopters' rotors do ([`RotorSpec`]) and
//! nacelles that tilt forward with speed ([`TiltSpec`]).
//!
//! # Provenance
//!
//! Not from the game, which has no Osprey mesh to read. The geometry is
//! the low-poly Osprey Brendan supplied (`osprey_avenger.obj`, an original
//! model scaled to the GTA Avenger's published 22.17 by 32.41 by 8.74 m;
//! [`crate::osprey_mesh`], generated from it by the `obj_mesh` example).
//! Here it is turned to the game's axes, stood on its wheels, and its
//! nacelles stood up to hover. Its colours (olive drab after the Avenger),
//! its mass (the Avenger's 18,000 kg), how far the nacelles tilt and how
//! fast are invented. Universal Petroleum's logo (`oc_logo_sign`, the
//! game's own texture, named by hash) takes the place of its side
//! roundels and is put on its tail fins: the Osprey is UP's alone.

use std::sync::OnceLock;

use m2_assets::model::{MATERIAL_CONSTANTS, Material};
use m2_assets::name_hash;
use m2_math::glam::{Mat4, Vec3};
use m2_render::{
    BufferUsage, IndexBufferDesc, IndexFormat, Topology, VertexBufferDesc, VertexFormat,
    VertexLayout, VertexSemantic,
};

use crate::model::{CpuModel, CpuPiece};
use crate::osprey_mesh::{CORNERS, PARTS, Part, SIZES, VERTICES};
use crate::tuning::VehicleTuning;
use crate::vehicle::{CatalogueEntry, HELICOPTER, RotorSpec, TiltSpec, VehicleSpec};

/// The Osprey's model hash, `global_veh_v22osprey`: the name its three
/// templates' `ModelName` gives (solved from the `global_veh_<name>`
/// pattern of the island's other vehicles; it hashes to `0x2e7cb0b5`).
pub const MODEL: u32 = 0x2e7c_b0b5;

/// The fuselage, wing, tails and gear.
const BODY: u16 = 0;
const LEFT_NACELLE: u16 = 1;
const LEFT_ROTOR: u16 = 2;
const RIGHT_NACELLE: u16 = 3;
const RIGHT_ROTOR: u16 = 4;
const NODES: usize = 5;

/// How far the nacelles tilt forward at full speed, radians (80°), and
/// the forward speed that brings them there, m/s (invented).
const FULL_TILT: f32 = 80.0 * std::f32::consts::PI / 180.0;
const TILT_SPEED: f32 = 45.0;
/// Its mass, kg: the Avenger's 18,000 (gtacars.net).
const MASS: f32 = 18_000.0;

/// The model's parts that tilt with a nacelle, and those that spin with
/// its proprotor; the rest is the body.
const NACELLE_PARTS: [&str; 3] = ["Tiltrotor_nacelle", "Nacelle_pivot_axis", "Engine_exhaust"];
const ROTOR_PARTS: [&str; 3] = ["Propeller_hub", "Hub_cap", "Rotor_blade"];

/// The materials, by index.
const PAINT: usize = 0;
const PAINT_LIGHT: usize = 1;
const DARK: usize = 2;
const BLADES: usize = 3;
const TYRE: usize = 4;
const GLASS: usize = 5;
const RED: usize = 6;
const GREEN: usize = 7;
const LOGO: usize = 8;
const MATERIALS: usize = 9;

/// The game's textures it draws with, by name hash.
const GLASS_TEXTURE: u32 = 0xb2a5_cef7; // global_veh_aircraftglass_black
const GLASS_SPECULAR: u32 = 0xdf41_0436; // global_veh_aircraftglass_sm
const DEFAULT_NORMAL: u32 = 0xa3cd_72a7; // global_defaultnormalmap
/// Universal Petroleum's logo, as its signs wear it (256 by 256, cut out by
/// its alpha).
const UP_LOGO: u32 = 0x5d46_e3f1; // oc_logo_sign
/// Whether the logo would read mirrored unless its texture is turned
/// (the renderer flips x, see [`CLOCKWISE`]).
const LOGO_FLIP: bool = false;

/// Material flags as the vehicles' own: a body (`0x0080`), glass blended
/// over what is behind (`0x0081`); see [`crate::model::flags`].
const BODY_FLAGS: u16 = 0x0080;
const GLASS_FLAGS: u16 = 0x0081;

/// Front faces wind clockwise seen from outside, as the game's do (see
/// the renderer's pipeline: anticlockwise on screen after its flip).
const CLOCKWISE: bool = true;

/// Whether `model` is the Osprey's.
pub fn is_osprey(model: u32) -> bool {
    model == MODEL
}

/// The Osprey to draw and fly, for catalogue entry `entry` (its kind is
/// taken to be a helicopter's whatever it says).
pub fn vehicle(entry: &CatalogueEntry) -> (VehicleSpec, CpuModel) {
    let l = layout();
    let cpu = model();
    let rest = l.rest.to_vec();
    let spec = VehicleSpec {
        model: MODEL,
        kind: HELICOPTER.to_owned(),
        label: entry.label.clone(),
        wheels: Vec::new(),
        bounds: cpu.bounds,
        seat: Some(l.seat),
        door: Some(l.door),
        tracked: false,
        road_wheels: Vec::new(),
        node_names: node_names().to_vec(),
        armed: Vec::new(),
        node_wheel: vec![None; NODES],
        node_rolls: Vec::new(),
        mass: MASS,
        turret: vec![false; NODES],
        pivot: None,
        muzzle: None,
        gun: vec![false; NODES],
        gun_pivot: None,
        muzzle_on_gun: false,
        elevation: (-10f32.to_radians(), 45f32.to_radians()),
        hull: l.hull,
        turret_box: None,
        tuning: VehicleTuning::default(),
        rotors: rotor_specs(&rest),
        tilts: tilt_specs(&rest),
        damage: None,
        rest,
    };
    (spec, cpu)
}

/// The Osprey for the delivery flights: its model, each node's rest
/// transform, and its proprotors as the game's rotors are listed.
pub fn flight() -> (CpuModel, Vec<Mat4>, Vec<m2_assets::vehicle::Rotor>) {
    let rest = layout().rest.to_vec();
    let rotors = rotor_specs(&rest)
        .into_iter()
        .map(|r| m2_assets::vehicle::Rotor {
            nodes: (0..NODES as u16)
                .filter(|&i| r.nodes[usize::from(i)])
                .collect(),
            centre: r.centre.to_array(),
            axis: r.axis.to_array(),
        })
        .collect();
    (model(), rest, rotors)
}

/// How far the nacelles tilt forward at forward speed `speed`, m/s:
/// upright hovering, down to [`FULL_TILT`] at [`TILT_SPEED`] (invented).
pub fn tilt_at(speed: f32) -> f32 {
    (speed / TILT_SPEED).clamp(0.0, 1.0) * FULL_TILT
}

fn node_names() -> [u32; NODES] {
    [
        name_hash("v22_body"),
        name_hash("v22_nacelle_l"),
        name_hash("v22_proprotor_l"),
        name_hash("v22_nacelle_r"),
        name_hash("v22_proprotor_r"),
    ]
}

/// The two proprotors: each turns about its nacelle's axis, the two
/// against each other as the aircraft's do.
fn rotor_specs(rest: &[Mat4]) -> Vec<RotorSpec> {
    [(LEFT_ROTOR, Vec3::Y), (RIGHT_ROTOR, Vec3::NEG_Y)]
        .into_iter()
        .map(|(node, axis)| RotorSpec {
            nodes: (0..NODES as u16).map(|i| i == node).collect(),
            centre: rest[usize::from(node)].w_axis.truncate(),
            axis,
        })
        .collect()
}

/// The nacelles, with their proprotors, tilt about their pivot axes.
fn tilt_specs(rest: &[Mat4]) -> Vec<TiltSpec> {
    [(LEFT_NACELLE, LEFT_ROTOR), (RIGHT_NACELLE, RIGHT_ROTOR)]
        .into_iter()
        .map(|(nacelle, rotor)| TiltSpec {
            nodes: (0..NODES as u16)
                .map(|i| i == nacelle || i == rotor)
                .collect(),
            pivot: rest[usize::from(nacelle)].w_axis.truncate(),
            axis: Vec3::X,
        })
        .collect()
}

/// The materials: two tones of olive paint, dark metal, blades, tyres,
/// tinted glass, the lights' red and green, and UP's logo.
fn materials() -> Vec<Material> {
    // An untextured material draws 0.6 grey times its direct and ambient
    // colours ([`crate::MaterialCache::material`]).
    let plain = |name: &str, rgb: [f32; 3], shine: f32, power: f32| {
        let mut constants = [0.0; MATERIAL_CONSTANTS];
        let tint = rgb.map(|c| c / 0.6);
        constants[0..3].copy_from_slice(&tint);
        constants[3..6].copy_from_slice(&[shine; 3]);
        constants[6..9].copy_from_slice(&tint);
        constants[12] = 1.0;
        constants[13] = power;
        constants[22] = 1.0;
        constants[23] = 1.0;
        Material {
            name_hash: name_hash(name),
            constants,
            flags: BODY_FLAGS,
            textures: vec![0, 0, 0],
            technique: name_hash("PgDiffSpecFP"),
            surface: name_hash("METAL"),
        }
    };
    let textured = |name: &str, textures: Vec<u32>| {
        let mut m = plain(name, [0.6; 3], 0.05, 16.0);
        m.textures = textures;
        m
    };
    let mut glass = textured(
        "v22_glass",
        vec![GLASS_TEXTURE, GLASS_SPECULAR, DEFAULT_NORMAL],
    );
    glass.constants[3..6].copy_from_slice(&[0.6; 3]);
    glass.constants[12] = 0.55;
    glass.constants[13] = 55.0;
    glass.flags = GLASS_FLAGS;
    glass.surface = name_hash("GLASS");
    let mut logo = textured("v22_up_logo", vec![UP_LOGO, 0, 0]);
    logo.technique = name_hash("PgDiffFP");
    // Olive drab, after the Avenger Brendan picked for it (2026-10-18).
    let out = vec![
        plain("v22_paint", [0.30, 0.31, 0.20], 0.14, 36.0),
        plain("v22_paint_light", [0.38, 0.39, 0.27], 0.14, 36.0),
        plain("v22_trim", [0.09, 0.10, 0.09], 0.12, 30.0),
        plain("v22_blades", [0.12, 0.13, 0.12], 0.10, 20.0),
        plain("v22_tyres", [0.04, 0.04, 0.04], 0.02, 8.0),
        glass,
        plain("v22_light_red", [0.72, 0.08, 0.06], 0.3, 40.0),
        plain("v22_light_green", [0.12, 0.58, 0.20], 0.3, 40.0),
        logo,
    ];
    debug_assert_eq!(out.len(), MATERIALS);
    out
}

/// The material an imported part draws with.
fn material_of(part: &Part) -> usize {
    match part.material {
        "airframe" => PAINT,
        "airframe_light" => PAINT_LIGHT,
        "glass" => GLASS,
        "rubber" => TYRE,
        "rotor" => BLADES,
        "marking" => LOGO,
        "light_red" => RED,
        "light_green" => GREEN,
        _ => DARK,
    }
}

/// Which node an imported part hangs from, by its name and side.
fn node_of(part: &Part, x: f32) -> u16 {
    let left = x < 0.0;
    if NACELLE_PARTS.contains(&part.name) {
        if left { LEFT_NACELLE } else { RIGHT_NACELLE }
    } else if ROTOR_PARTS.contains(&part.name) {
        if left { LEFT_ROTOR } else { RIGHT_ROTOR }
    } else {
        BODY
    }
}

/// A file vertex in the game's axes: the file has z up and the nose along
/// +y; the game has y up and the nose along +z. `(-x, z, y)` is a turn,
/// not a mirror, so the faces keep their winding.
fn game_axes([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3::new(-x, z, y)
}

/// A part's polygons, each its corners in the game's axes, before any
/// nacelle is stood up or the whole lifted onto its wheels.
fn polygons(part: &Part) -> Vec<Vec<Vec3>> {
    let mut start: usize = SIZES[..part.polygons.start]
        .iter()
        .map(|&s| usize::from(s))
        .sum();
    part.polygons
        .clone()
        .map(|p| {
            let n = usize::from(SIZES[p]);
            let corners = CORNERS[start..start + n]
                .iter()
                .map(|&i| game_axes(VERTICES[usize::from(i)]))
                .collect();
            start += n;
            corners
        })
        .collect()
}

fn centroid(points: impl IntoIterator<Item = Vec3>) -> Vec3 {
    let (sum, n) = points
        .into_iter()
        .fold((Vec3::ZERO, 0.0f32), |(s, n), p| (s + p, n + 1.0));
    if n > 0.0 { sum / n } else { Vec3::ZERO }
}

fn bounds_of(points: impl IntoIterator<Item = Vec3>) -> (Vec3, Vec3) {
    points.into_iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    )
}

/// Where the imported model's parts go, worked out once from its
/// geometry: each node's rest transform, how far it is lifted to stand on
/// its wheels, each nacelle's pivot and the turn that stands it up, and
/// the markers.
struct Layout {
    rest: [Mat4; NODES],
    /// Added to every vertex: its wheels' bottoms to the ground, its
    /// fuselage centred across (the file's is 0.42 m off).
    lift: Vec3,
    /// Per side (left, right): the pivot, file pose.
    pivots: [Vec3; 2],
    hull: (Vec3, Vec3),
    seat: Vec3,
    door: Vec3,
    /// The tail fins' boxes, for the logos.
    fins: Vec<(Vec3, Vec3)>,
}

/// The turn standing a nacelle up from the file's forward-flight pose:
/// its nose (+z) to the sky (+y).
fn hover() -> Mat4 {
    Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

fn layout() -> &'static Layout {
    static LAYOUT: OnceLock<Layout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        let named = |name: &str| -> Vec<Vec3> {
            PARTS
                .iter()
                .filter(|p| p.name == name)
                .flat_map(|p| polygons(p).into_iter().flatten())
                .collect()
        };
        let tyres: Vec<Vec3> = ["Main_tire", "Nose_tire"]
            .iter()
            .flat_map(|n| named(n))
            .collect();
        // Stood on its wheels, its fuselage centred across.
        let (flo, fhi) = bounds_of(named("Fuselage_shell"));
        let lift = Vec3::new(-(flo.x + fhi.x) * 0.5, -bounds_of(tyres).0.y, 0.0);
        let side = |points: &[Vec3], left: bool| -> Vec<Vec3> {
            points
                .iter()
                .copied()
                .filter(|p| (p.x < 0.0) == left)
                .collect()
        };
        let axes = named("Nacelle_pivot_axis");
        // The proprotor turns about its hub cap, in the disc of its blades.
        let hubs = named("Hub_cap");
        let pivots = [centroid(side(&axes, true)), centroid(side(&axes, false))];
        let stood =
            |pivot: Vec3| Mat4::from_translation(pivot) * hover() * Mat4::from_translation(-pivot);
        let mut rest = [Mat4::IDENTITY; NODES];
        for (k, left) in [true, false].into_iter().enumerate() {
            let pivot = pivots[k];
            let hub = stood(pivot).transform_point3(centroid(side(&hubs, left)));
            let (nacelle, rotor) = if left {
                (LEFT_NACELLE, LEFT_ROTOR)
            } else {
                (RIGHT_NACELLE, RIGHT_ROTOR)
            };
            rest[usize::from(nacelle)] = Mat4::from_translation(pivot + lift);
            rest[usize::from(rotor)] = Mat4::from_translation(hub + lift);
        }
        let (lo, hi) = (flo, fhi);
        let glazing = named("Cockpit_side_glazing");
        let cockpit = centroid(glazing.iter().copied());
        let (glo, ghi) = bounds_of(glazing);
        let door = centroid(named("Sliding_cabin_door"));
        let fins = [true, false]
            .into_iter()
            .map(|left| {
                let (a, b) = bounds_of(side(&named("Vertical_tail_fin"), left));
                (a + lift, b + lift)
            })
            .collect();
        Layout {
            rest,
            lift,
            pivots,
            hull: (lo + lift, hi + lift),
            // The pilot's seat: the right of the cockpit, low in its
            // glazing.
            seat: Vec3::new(
                (ghi.x - glo.x) * 0.2,
                glo.y + (ghi.y - glo.y) * 0.1,
                cockpit.z,
            ) + lift,
            // Outside the cabin door, on the ground.
            door: Vec3::new(door.x + lift.x + (door.x + lift.x).signum(), 0.0, door.z),
            fins,
        }
    })
}

/// Triangles in one node's space, by material.
#[derive(Default)]
struct Mesh {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    indices: [Vec<u16>; MATERIALS],
}

impl Mesh {
    fn vertex(&mut self, p: Vec3, n: Vec3, uv: [f32; 2]) -> u16 {
        self.positions.push(p);
        self.normals.push(n.normalize_or_zero());
        self.uvs.push(uv);
        (self.positions.len() - 1) as u16
    }

    /// A triangle facing `outward`.
    fn triangle(&mut self, m: usize, [a, b, c]: [u16; 3], outward: Vec3) {
        let [pa, pb, pc] = [a, b, c].map(|i| self.positions[usize::from(i)]);
        let facing = (pb - pa).cross(pc - pa).dot(outward) > 0.0;
        // Anticlockwise from outside when the cross product faces out.
        let tri = if facing != CLOCKWISE {
            [a, b, c]
        } else {
            [a, c, b]
        };
        self.indices[m].extend(tri);
    }

    /// A flat polygon, its corners anticlockwise seen from outside (as
    /// the file winds them), fanned into triangles, each corner at its
    /// texture place `uv`.
    fn polygon(&mut self, m: usize, corners: &[Vec3], uv: impl Fn(Vec3) -> [f32; 2]) {
        // Newell's normal: right for any flat polygon, convex or not.
        let mut n = Vec3::ZERO;
        for (i, &a) in corners.iter().enumerate() {
            let b = corners[(i + 1) % corners.len()];
            n += Vec3::new(
                (a.y - b.y) * (a.z + b.z),
                (a.z - b.z) * (a.x + b.x),
                (a.x - b.x) * (a.y + b.y),
            );
        }
        if n.length_squared() < 1e-12 {
            return;
        }
        let v: Vec<u16> = corners.iter().map(|&p| self.vertex(p, n, uv(p))).collect();
        for k in 1..v.len() - 1 {
            self.triangle(m, [v[0], v[k], v[k + 1]], n);
        }
    }

    /// Its triangles as an uploadable piece hanging from `node`, which
    /// rests at `rest`; `None` when it has none.
    fn piece(self, node: u16, rest: Mat4) -> Option<CpuPiece> {
        let mut indices = Vec::new();
        let mut ranges = Vec::new();
        for (m, list) in self.indices.iter().enumerate() {
            if list.is_empty() {
                continue;
            }
            ranges.push((indices.len() as u32, list.len() as u32, m));
            indices.extend_from_slice(list);
        }
        if indices.is_empty() {
            return None;
        }
        let layout = VertexLayout::packed(&[
            (VertexSemantic::Position, VertexFormat::Float3),
            (VertexSemantic::Normal, VertexFormat::Float3),
            (VertexSemantic::TexCoord(0), VertexFormat::Float2),
        ])
        .expect("a fixed, valid layout");
        let mut vertices = Vec::with_capacity(self.positions.len() * 32);
        for ((p, n), uv) in self.positions.iter().zip(&self.normals).zip(&self.uvs) {
            for f in [p.x, p.y, p.z, n.x, n.y, n.z, uv[0], uv[1]] {
                vertices.extend_from_slice(&f.to_le_bytes());
            }
        }
        let (lo, hi) = bounds_of(self.positions.iter().copied());
        Some(CpuPiece {
            vertex_desc: VertexBufferDesc {
                layout,
                vertex_count: self.positions.len() as u32,
                usage: BufferUsage::Static,
            },
            vertices,
            index_desc: IndexBufferDesc {
                format: IndexFormat::U16,
                index_count: indices.len() as u32,
                usage: BufferUsage::Static,
            },
            indices: indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
            local: rest,
            ranges,
            topology: Topology::TriangleList,
            members: Vec::new(),
            centre: rest.transform_point3((lo + hi) * 0.5),
            bones: Vec::new(),
            node: Some(node),
            at: node,
        })
    }
}

/// The logo's texture place for a point on a side face whose box is
/// `(lo, hi)` (model space), facing `side` (-1 or +1 along x): upright,
/// reading nose to tail as seen from that side.
fn logo_uv(p: Vec3, (lo, hi): (Vec3, Vec3), side: f32) -> [f32; 2] {
    let along = (p.z - lo.z) / (hi.z - lo.z).max(1e-4);
    let up = (p.y - lo.y) / (hi.y - lo.y).max(1e-4);
    // Seen from outside the +x side, the nose is on the left.
    let u = if (side > 0.0) != LOGO_FLIP {
        1.0 - along
    } else {
        along
    };
    [u, 1.0 - up]
}

/// Each node's geometry in its own space.
fn meshes() -> [Mesh; NODES] {
    let l = layout();
    let mut meshes: [Mesh; NODES] = Default::default();
    for part in &PARTS {
        let polys = polygons(part);
        let all: Vec<Vec3> = polys.iter().flatten().copied().collect();
        let x = centroid(all.iter().copied()).x;
        let node = node_of(part, x);
        let material = material_of(part);
        // A nacelle's parts are stood up about its pivot.
        let turn = match node {
            BODY => Mat4::IDENTITY,
            _ => {
                let pivot = l.pivots[usize::from(x >= 0.0)];
                Mat4::from_translation(pivot) * hover() * Mat4::from_translation(-pivot)
            }
        };
        let to_node = l.rest[usize::from(node)].inverse() * Mat4::from_translation(l.lift) * turn;
        let placed: Vec<Vec<Vec3>> = polys
            .iter()
            .map(|c| c.iter().map(|&p| to_node.transform_point3(p)).collect())
            .collect();
        let bounds = bounds_of(placed.iter().flatten().copied());
        let side = x.signum();
        for corners in &placed {
            if material == LOGO {
                meshes[usize::from(node)].polygon(material, corners, |p| logo_uv(p, bounds, side));
            } else {
                meshes[usize::from(node)].polygon(material, corners, |_| [0.0, 0.0]);
            }
        }
    }
    // UP's logo on both faces of each tail fin, upright.
    for &(lo, hi) in &l.fins {
        let (cy, cz) = ((lo.y + hi.y) * 0.5, (lo.z + hi.z) * 0.5);
        let size = (hi.y - lo.y).min(hi.z - lo.z) * 0.45;
        let (y0, y1, z0, z1) = (cy - size, cy + size, cz - size, cz + size);
        for (face, x) in [(1.0, hi.x + 0.01), (-1.0, lo.x - 0.01)] {
            // Wound anticlockwise seen from the side it faces: this order
            // faces -x, reversed +x.
            let mut corners = vec![
                Vec3::new(x, y0, z0),
                Vec3::new(x, y0, z1),
                Vec3::new(x, y1, z1),
                Vec3::new(x, y1, z0),
            ];
            if face > 0.0 {
                corners.reverse();
            }
            let bounds = (Vec3::new(x, y0, z0), Vec3::new(x, y1, z1));
            meshes[usize::from(BODY)].polygon(LOGO, &corners, |p| logo_uv(p, bounds, face));
        }
    }
    meshes
}

/// The whole stand-in model.
pub fn model() -> CpuModel {
    let l = layout();
    let pieces: Vec<CpuPiece> = meshes()
        .into_iter()
        .enumerate()
        .filter_map(|(node, mesh)| mesh.piece(node as u16, l.rest[node]))
        .collect();
    // Its bounds with the proprotors turning: the body's and each hub's
    // disc.
    let mut bounds = bounds_of(pieces.iter().flat_map(|p| {
        p.vertices.chunks_exact(32).map(|v| {
            let f = |i: usize| f32::from_le_bytes(v[i * 4..i * 4 + 4].try_into().unwrap());
            p.local.transform_point3(Vec3::new(f(0), f(1), f(2)))
        })
    }));
    for rotor in [LEFT_ROTOR, RIGHT_ROTOR] {
        let hub = l.rest[usize::from(rotor)].w_axis.truncate();
        let reach = blade_reach();
        bounds.0 = bounds.0.min(hub - Vec3::new(reach, 0.0, reach));
        bounds.1 = bounds.1.max(hub + Vec3::new(reach, 0.0, reach));
    }
    CpuModel {
        hash: MODEL,
        pieces,
        materials: materials(),
        bounds,
        skipped_groups: 0,
        collision: None,
        members: Vec::new(),
        hidden: [Vec::new(), Vec::new()],
        textures: Vec::new(),
        tiny_members: Vec::new(),
        sinks: Vec::new(),
        breakup: None,
        parts: None,
        looks: None,
    }
}

/// How far a proprotor's blades reach from its hub.
fn blade_reach() -> f32 {
    let m = &meshes()[usize::from(RIGHT_ROTOR)];
    m.positions
        .iter()
        .map(|p| Vec3::new(p.x, 0.0, p.z).length())
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osprey() -> (VehicleSpec, CpuModel) {
        vehicle(&CatalogueEntry {
            model: MODEL,
            kind: HELICOPTER.into(),
            label: "v22".into(),
        })
    }

    #[test]
    fn the_osprey_has_a_body_two_nacelles_and_two_proprotors() {
        let cpu = model();
        assert_eq!(cpu.pieces.len(), NODES);
        assert_eq!(cpu.materials.len(), MATERIALS);
        for p in &cpu.pieces {
            let stride = p.vertex_desc.layout.stride() as usize;
            assert_eq!(
                p.vertices.len(),
                stride * p.vertex_desc.vertex_count as usize
            );
            assert_eq!(p.indices.len(), p.index_desc.index_count as usize * 2);
            assert_eq!(p.index_desc.index_count % 3, 0);
            let count = p.vertex_desc.vertex_count;
            for pair in p.indices.chunks_exact(2) {
                assert!(u32::from(u16::from_le_bytes([pair[0], pair[1]])) < count);
            }
            let ranged: u32 = p.ranges.iter().map(|r| r.1).sum();
            assert_eq!(ranged, p.index_desc.index_count);
            assert!(p.ranges.iter().all(|r| r.2 < MATERIALS));
        }
        // Every part of the file is drawn, the roundels as UP's logo.
        let logo = cpu.pieces[0].ranges.iter().any(|r| r.2 == LOGO);
        assert!(logo);
    }

    #[test]
    fn it_is_sized_as_the_avenger_and_stands_on_its_wheels() {
        let (spec, cpu) = osprey();
        assert!(spec.is_helicopter());
        // 22.17 m nose to tail, 32.41 m across the proprotors' discs.
        let length = cpu.bounds.1.z - cpu.bounds.0.z;
        assert!((21.0..23.5).contains(&length), "{length}");
        let span = cpu.bounds.1.x - cpu.bounds.0.x;
        // (The file's blades reach a little further than the Avenger's.)
        assert!((31.0..35.0).contains(&span), "{span}");
        assert!(cpu.bounds.0.y.abs() < 0.05, "{}", cpu.bounds.0.y);
        // The two hubs, one each side, high over the wing tips, turning
        // against each other.
        let hubs: Vec<Vec3> = spec.rotors.iter().map(|r| r.centre).collect();
        assert_eq!(hubs.len(), 2);
        assert!(hubs[0].x < -5.0 && hubs[1].x > 5.0, "{hubs:?}");
        assert!((hubs[0].y - hubs[1].y).abs() < 0.05);
        assert!(hubs[0].y > spec.hull.1.y, "{hubs:?} {:?}", spec.hull);
        assert!(spec.rotors[0].axis.dot(spec.rotors[1].axis) < 0.0);
        // The pilot sits in the nose, inside the hull.
        let seat = spec.seat.unwrap();
        assert!(
            seat.z > spec.hull.0.z + (spec.hull.1.z - spec.hull.0.z) * 0.6,
            "{seat}"
        );
        assert!(spec.door.is_some());
    }

    #[test]
    fn the_nacelles_tilt_with_speed() {
        assert_eq!(tilt_at(0.0), 0.0);
        assert!(tilt_at(TILT_SPEED * 0.5) > 0.0);
        assert!((tilt_at(500.0) - FULL_TILT).abs() < 1e-6);
        assert_eq!(tilt_at(-10.0), 0.0);
        // Tilted fully, a hub moves forward of its pivot.
        let (spec, _) = osprey();
        let mut nodes = spec.rest.clone();
        spec.tilt_parts(&mut nodes, FULL_TILT);
        let hub = nodes[usize::from(RIGHT_ROTOR)].w_axis.truncate();
        let pivot = spec.tilts[1].pivot;
        assert!(hub.z > pivot.z + 0.5, "{hub} {pivot}");
    }

    #[test]
    fn its_triangles_face_the_way_the_game_winds_them() {
        for mesh in meshes() {
            for list in &mesh.indices {
                for t in list.chunks_exact(3) {
                    let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[usize::from(i)]);
                    let n = mesh.normals[usize::from(t[0])];
                    let cross = (b - a).cross(c - a);
                    if cross.length() < 1e-6 {
                        continue;
                    }
                    assert_eq!(cross.dot(n) < 0.0, CLOCKWISE);
                }
            }
        }
    }
}
