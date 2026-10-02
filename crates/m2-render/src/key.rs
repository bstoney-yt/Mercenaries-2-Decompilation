//! 64-bit draw sort keys.
//!
//! The engine's `PgPrimitive` class has `AssignKeys`, `Sort` and
//! `SubmitToGPU` steps (names from the executable's readable strings only).
//! The bit layout below is a clean-room design following common practice,
//! not a recovery of the original key format.
//!
//! Most significant bits first:
//!
//! | bits   | opaque                 | translucent               |
//! |--------|------------------------|---------------------------|
//! | 63..56 | layer                  | layer                     |
//! | 55     | 0                      | 1                         |
//! | 54..39 | material               | inverted depth (54..31)   |
//! | 38..15 | depth (front to back)  | material (30..15)         |
//! | 14..0  | user tiebreak          | user tiebreak             |
//!
//! So within a layer, opaque draws come first and are grouped by material to
//! cut state changes, then nearest first to help early depth rejection;
//! translucent draws follow, farthest first so blending composes correctly.

/// Coarse ordering bucket; lower layers draw first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Layer(pub u8);

impl Layer {
    /// Sky, distant backdrops.
    pub const BACKGROUND: Self = Self(0x10);
    /// Terrain, models, everything depth-tested.
    pub const WORLD: Self = Self(0x40);
    /// Screen-space effects drawn after the world.
    pub const POST: Self = Self(0xA0);
    /// HUD and menus.
    pub const OVERLAY: Self = Self(0xC0);
}

/// Identifies the shader and state block a primitive draws with. Only the low
/// 16 bits reach the key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialId(pub u32);

const DEPTH_BITS: u32 = 24;
const DEPTH_MAX: u32 = (1 << DEPTH_BITS) - 1;
const USER_BITS: u32 = 15;
const USER_MAX: u16 = (1 << USER_BITS) - 1;

/// A packed sort key. Sorting ascending gives draw order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SortKey(pub u64);

/// The inputs to a [`SortKey`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyFields {
    pub layer: Layer,
    pub translucent: bool,
    pub material: MaterialId,
    /// Normalised view depth, `0.0` at the near plane and `1.0` at the far
    /// plane. Values outside are clamped.
    pub depth: f32,
    /// Final tiebreak; only the low 15 bits are kept.
    pub user: u16,
}

impl SortKey {
    pub fn new(f: KeyFields) -> Self {
        let layer = u64::from(f.layer.0) << 56;
        let material = u64::from(f.material.0 & 0xFFFF);
        let depth = u64::from(quantize_depth(f.depth));
        let user = u64::from(f.user & USER_MAX);
        let body = if f.translucent {
            (1 << 55) | (u64::from(DEPTH_MAX) - depth) << 31 | material << 15
        } else {
            material << 39 | depth << 15
        };
        Self(layer | body | user)
    }

    pub fn layer(self) -> Layer {
        Layer((self.0 >> 56) as u8)
    }

    pub fn is_translucent(self) -> bool {
        self.0 & (1 << 55) != 0
    }

    pub fn material(self) -> MaterialId {
        let shift = if self.is_translucent() { 15 } else { 39 };
        MaterialId(((self.0 >> shift) & 0xFFFF) as u32)
    }

    /// The quantised depth, `0..=2^24-1`.
    pub fn depth_bits(self) -> u32 {
        if self.is_translucent() {
            DEPTH_MAX - ((self.0 >> 31) as u32 & DEPTH_MAX)
        } else {
            (self.0 >> 15) as u32 & DEPTH_MAX
        }
    }

    pub fn user(self) -> u16 {
        self.0 as u16 & USER_MAX
    }
}

fn quantize_depth(depth: f32) -> u32 {
    // `clamp` passes NaN through; the saturating float-to-int cast then maps
    // it to 0.
    (depth.clamp(0.0, 1.0) * DEPTH_MAX as f32) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(layer: Layer, translucent: bool, material: u32, depth: f32) -> SortKey {
        SortKey::new(KeyFields {
            layer,
            translucent,
            material: MaterialId(material),
            depth,
            user: 0,
        })
    }

    #[test]
    fn fields_round_trip() {
        for translucent in [false, true] {
            let k = SortKey::new(KeyFields {
                layer: Layer::WORLD,
                translucent,
                material: MaterialId(0x1234),
                depth: 0.5,
                user: 7,
            });
            assert_eq!(k.layer(), Layer::WORLD);
            assert_eq!(k.is_translucent(), translucent);
            assert_eq!(k.material(), MaterialId(0x1234));
            assert_eq!(k.depth_bits(), quantize_depth(0.5));
            assert_eq!(k.user(), 7);
        }
    }

    #[test]
    fn layer_dominates() {
        assert!(key(Layer::BACKGROUND, true, 0xFFFF, 0.0) < key(Layer::WORLD, false, 0, 1.0));
    }

    #[test]
    fn opaque_before_translucent() {
        assert!(key(Layer::WORLD, false, 0xFFFF, 1.0) < key(Layer::WORLD, true, 0, 1.0));
    }

    #[test]
    fn opaque_groups_by_material_then_front_to_back() {
        assert!(key(Layer::WORLD, false, 1, 0.9) < key(Layer::WORLD, false, 2, 0.1));
        assert!(key(Layer::WORLD, false, 1, 0.1) < key(Layer::WORLD, false, 1, 0.9));
    }

    #[test]
    fn translucent_back_to_front() {
        assert!(key(Layer::WORLD, true, 9, 0.9) < key(Layer::WORLD, true, 1, 0.1));
    }

    #[test]
    fn depth_clamps() {
        assert_eq!(quantize_depth(-3.0), 0);
        assert_eq!(quantize_depth(f32::NAN), 0);
        assert_eq!(quantize_depth(4.0), DEPTH_MAX);
    }
}
