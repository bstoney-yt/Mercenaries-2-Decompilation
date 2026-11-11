//! The 360's collision layers: which of its 32 layers a body's shapes go
//! on, which layers meet, and which a query sees.
//!
//! From the 360 code:
//! - **Which layers meet:** the world's `hkpGroupFilter` is set up in
//!   0x82164A58 ([`FILTER`], one 32-bit row a layer; 0x824CD1A0 tests a
//!   pair by it), the code run and read off whole.
//! - **A shape's layer:** 0x8216C308 picks it from its `PHY2` flags (word 4,
//!   [`m2_assets::physics::Shape::flags`]) in the eight its body's class
//!   gives ([`layer_of`]). [`BUILDING`] and the like are 0x821697D0's
//!   presets; a fence or a wall is `_BuildingPhysics` (0x82188890).
//! - **What a query sees:** 0x827F8550: a body counts when its layer is 0
//!   or the query's mask has its bit ([`seen_by`]). The path search's
//!   sweeps and ground rays ask [`PATH_MASK`].
//!
//! Inferred from the filter and the data, not named anywhere: flag bit 1
//! makes a shape solid to what moves (every layer without it meets only
//! 0, 27 and 28), bit 2 stops sight lines ([`SIGHT_MASK`] has those layers
//! and not the others), and bit 5 stands in an AI's way ([`PATH_MASK`]
//! has only those). Bits 5 and 6 alone are on the shapes of the two
//! `..._aicollision` bridges, on layer 30, which nothing but layer 0
//! meets and the path search sees: shapes for AI drivers only.
//!
//! So what an AI drives through is authored shape by shape: a chain-link
//! or barbed-wire fence (flags 7) is solid to a tank and no obstacle to
//! its path, and it just breaks it.

/// Which layers each layer meets, bit by bit (0x82164A58).
pub const FILTER: [u32; 32] = [
    0xffff_ffff,
    0x1eb5_b36d,
    0x14f5_b36f,
    0x1cf5_b36f,
    0x1800_0001,
    0x14f5_b36f,
    0x1cf5_b36f,
    0x1800_0001,
    0x2efd_bf6f,
    0x2efd_bf6f,
    0x1020_0301,
    0x0e3d_bb01,
    0x24f5_bb6f,
    0x1cf5_bb6f,
    0x0800_0001,
    0x24f5_bb6f,
    0x1cf5_bb6f,
    0x0800_0001,
    0x28f5_bb6f,
    0x0060_0b01,
    0x28c5_bb6f,
    0x2aed_bf6f,
    0x023d_b36d,
    0x06b5_b36f,
    0x0000_0001,
    0x28e0_0b03,
    0x0081_bb6f,
    0x2237_6bdb,
    0x1001_24ff,
    0x0a34_9301,
    0x0000_0001,
    0x0000_0001,
];

/// What a path search's sweeps (0x822E6FD8) and ground rays (0x822E4120)
/// see.
pub const PATH_MASK: u32 = 0x6008_651e;
/// What a vehicle's look just ahead sees (0x822C8CB8, 0x822C8770).
pub const NOSE_MASK: u32 = 0x6228_651e;
/// What a gunner's sight lines see (0x822C95D8, 0x822E7258).
pub const SIGHT_MASK: u32 = 0x223f_6fda;

/// The layers a body's class gives its shapes (0x821697D0): by the shape's
/// flags, as [`layer_of`] reads them; -1 for none of its own.
pub type Layers = [i32; 8];
/// Preset 3: a `_BuildingPhysics` body (0x82188890).
pub const BUILDING: Layers = [4, 7, 3, 6, 2, 5, 19, -1];
/// Preset 0: a `_CarPhysicsV2` or `_TankPhysics` body (0x82189448,
/// 0x82194520).
pub const VEHICLE: Layers = [8, 9, 8, 9, 8, 9, 11, 11];
/// Preset 1: a `_PropPhysics` or `_DebrisPhysics` body (0x8218EBF8,
/// 0x8218B728).
pub const PROP: Layers = [14, 17, 13, 16, 12, 15, 19, -1];
/// Preset 2: such a body once 0x82155328 says so (a loose prop).
pub const LOOSE_PROP: Layers = [14, 17, 18, -1, -1, -1, -1, -1];
/// Preset 4: a `_BoatPhysics` body (0x82188000).
pub const BOAT: Layers = [29, 29, 8, 9, 28, 28, 11, 11];

/// A shape's kind (`PHY2` word 11) that takes no leaves' or AI-only layer.
const MESH: u32 = 4;

/// The layer of a shape of `flags` and `kind` on a body of `layers`
/// (0x8216C308): no flags, the third; bit 4, layer 0; bit 6, layer 30;
/// bit 3, the seventh (those two, for any kind but a mesh, which takes the
/// third); else by bits 2, 1 and 5.
pub fn layer_of(layers: &Layers, flags: u32, kind: u32) -> u32 {
    let third = layers[2].max(0) as u32;
    if flags == 0 {
        return third;
    }
    let (bit2, bit1, bit5) = (flags >> 2 & 1, flags >> 1 & 1, flags >> 5 & 1);
    if flags >> 4 & 1 != 0 {
        return 0;
    }
    if flags >> 6 & 1 != 0 {
        return if kind == MESH { third } else { 30 };
    }
    let pick = if flags >> 3 & 1 != 0 {
        if kind == MESH {
            return third;
        }
        layers[6]
    } else {
        match (bit2, bit1, bit5) {
            (1, 1, 1) => layers[2],
            (1, 1, 0) => layers[3],
            (1, 0, 1) => layers[0],
            (1, 0, 0) => layers[1],
            (0, 1, 1) => layers[4],
            (0, 1, 0) => layers[5],
            _ => return 24,
        }
    };
    if pick == -1 { third } else { pick as u32 }
}

/// Whether bodies on layers `a` and `b` meet (0x824CD1A0).
pub fn meet(a: u32, b: u32) -> bool {
    FILTER[(a & 31) as usize] >> (b & 31) & 1 != 0
}

/// The layers of a vehicle's hull and turret ([`VEHICLE`]'s).
pub const VEHICLE_LAYERS: [u32; 2] = [8, 9];

/// Whether a vehicle's hull meets a shape on `layer` ([`meet`] with
/// [`VEHICLE_LAYERS`]).
pub fn meets_vehicles(layer: u32) -> bool {
    VEHICLE_LAYERS.iter().any(|&v| meet(v, layer))
}

/// Whether a query of `mask` sees a body on `layer` (0x827F8550).
pub fn seen_by(mask: u32, layer: u32) -> bool {
    layer == 0 || mask >> (layer & 31) & 1 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_is_symmetric() {
        for a in 0..32 {
            for b in 0..32 {
                assert_eq!(meet(a, b), meet(b, a), "{a} {b}");
            }
        }
    }

    // 0x8216C308 on the flags the data has.
    #[test]
    fn a_building_shape_takes_its_layer_from_its_flags() {
        for (flags, layer) in [
            (0x27, 3),
            (0x07, 6),
            (0x23, 2),
            (0x03, 5),
            (0x25, 4),
            (0x05, 7),
            (0x00, 3),
        ] {
            assert_eq!(layer_of(&BUILDING, flags, 3), layer, "{flags:#x}");
        }
        // Leaves, and the bridges' AI-only shapes.
        assert_eq!(layer_of(&BUILDING, 0x28, 3), 19);
        assert_eq!(layer_of(&BUILDING, 0x60, 3), 30);
        assert_eq!(layer_of(&BUILDING, 0x60, 4), 3);
        assert_eq!(layer_of(&LOOSE_PROP, 0x27, 3), 18);
        assert_eq!(layer_of(&LOOSE_PROP, 0x07, 3), 18);
    }

    // A chain fence (flags 7) stops a tank but not its path; a wooden one
    // (0x27) stops both.
    #[test]
    fn a_tank_meets_a_chain_fence_but_its_path_does_not() {
        let tank = layer_of(&VEHICLE, 0x27, 3);
        let chain = layer_of(&BUILDING, 0x07, 3);
        let wood = layer_of(&BUILDING, 0x27, 3);
        assert!(meet(tank, chain) && meet(tank, wood));
        assert!(!seen_by(PATH_MASK, chain));
        assert!(seen_by(PATH_MASK, wood));
        assert!(seen_by(PATH_MASK, tank));
        assert!(!seen_by(NOSE_MASK, chain));
        assert!(seen_by(SIGHT_MASK, chain));
    }

    // The bridges' AI shapes: only paths see them, nothing that moves meets
    // them.
    #[test]
    fn layer_30_is_for_paths_alone() {
        assert!(seen_by(PATH_MASK, 30));
        assert!(!meet(8, 30));
        // Without bit 1, nothing that moves meets a shape.
        for shape in [4, 7, 14, 17] {
            assert!(!meet(8, shape));
        }
    }
}
