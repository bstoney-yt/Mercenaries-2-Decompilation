//! Math types.
//!
//! The original engine stores vectors and matrices in D3DX layout: tightly
//! packed `f32`s, matrices row-major with row vectors (`v * M`). The `Raw*`
//! types mirror that layout for use inside `#[repr(C)]` structs; game logic
//! converts to `glam` (column-major, column vectors) at the boundary instead
//! of embedding glam types in raw structs.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use m2_layout::assert_layout;

pub use glam;

#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub struct RawVec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub struct RawVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub struct RawVec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub struct RawQuat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// Row-major 4x4 matrix, `m[row][col]`, as `D3DXMATRIX`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub struct RawMatrix44 {
    pub m: [[f32; 4]; 4],
}

assert_layout!(RawVec2, size = 8);
assert_layout!(RawVec3, size = 12, { x: 0, y: 4, z: 8 });
assert_layout!(RawVec4, size = 16);
assert_layout!(RawQuat, size = 16, { w: 12 });
assert_layout!(RawMatrix44, size = 64);

impl From<RawVec2> for Vec2 {
    fn from(v: RawVec2) -> Self {
        Vec2::new(v.x, v.y)
    }
}

impl From<Vec2> for RawVec2 {
    fn from(v: Vec2) -> Self {
        Self { x: v.x, y: v.y }
    }
}

impl From<RawVec3> for Vec3 {
    fn from(v: RawVec3) -> Self {
        Vec3::new(v.x, v.y, v.z)
    }
}

impl From<Vec3> for RawVec3 {
    fn from(v: Vec3) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }
}

impl From<RawVec4> for Vec4 {
    fn from(v: RawVec4) -> Self {
        Vec4::new(v.x, v.y, v.z, v.w)
    }
}

impl From<Vec4> for RawVec4 {
    fn from(v: Vec4) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
            w: v.w,
        }
    }
}

impl From<RawQuat> for Quat {
    fn from(q: RawQuat) -> Self {
        Quat::from_xyzw(q.x, q.y, q.z, q.w)
    }
}

impl From<Quat> for RawQuat {
    fn from(q: Quat) -> Self {
        Self {
            x: q.x,
            y: q.y,
            z: q.z,
            w: q.w,
        }
    }
}

/// A row-major, row-vector D3DX matrix and a column-major, column-vector glam
/// matrix describe the same transform with the same 16 floats in the same
/// order, so the conversion is a straight copy: D3DX row `i` becomes glam
/// column `i`.
impl From<RawMatrix44> for Mat4 {
    fn from(m: RawMatrix44) -> Self {
        Mat4::from_cols_array_2d(&m.m)
    }
}

impl From<Mat4> for RawMatrix44 {
    fn from(m: Mat4) -> Self {
        Self {
            m: m.to_cols_array_2d(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_lives_in_row_3() {
        // D3DXMatrixTranslation puts the offset in _41.._43.
        let mut raw = RawMatrix44 {
            m: Mat4::IDENTITY.to_cols_array_2d(),
        };
        raw.m[3][0] = 1.0;
        raw.m[3][1] = 2.0;
        raw.m[3][2] = 3.0;

        let m = Mat4::from(raw);
        assert_eq!(m.transform_point3(Vec3::ZERO), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(RawMatrix44::from(m), raw);
    }

    #[test]
    fn vec3_round_trip() {
        let v = Vec3::new(1.0, -2.0, 3.5);
        assert_eq!(Vec3::from(RawVec3::from(v)), v);
    }
}
