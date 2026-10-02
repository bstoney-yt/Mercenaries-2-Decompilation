//! D3DX-equivalent operations on the `Raw*` types.
//!
//! Clean-room: these follow the documented behaviour of the D3DX 9 math
//! functions, not code recovered from the game, so they have no entries in
//! `symbols/`. Each method names the D3DX function it matches.
//!
//! D3DX uses row vectors (`v * M`), so a product `a * b` applies `a` first.
//! Every method keeps that order. Internally the work is done in glam, which
//! sees the same 16 floats as the transpose (see the `Mat4` conversion), so a
//! D3DX product `a * b` is computed as the glam product `b * a`.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::{RawMatrix44, RawQuat, RawVec2, RawVec3, RawVec4};

impl RawVec2 {
    /// `D3DXVec2Dot`.
    pub fn dot(self, rhs: Self) -> f32 {
        Vec2::from(self).dot(rhs.into())
    }

    /// `D3DXVec2Length`.
    pub fn length(self) -> f32 {
        Vec2::from(self).length()
    }

    /// `D3DXVec2LengthSq`.
    pub fn length_sq(self) -> f32 {
        Vec2::from(self).length_squared()
    }

    /// `D3DXVec2Normalize`. A zero-length vector normalizes to zero.
    pub fn normalize(self) -> Self {
        Vec2::from(self).normalize_or_zero().into()
    }
}

impl RawVec3 {
    /// `D3DXVec3Dot`.
    pub fn dot(self, rhs: Self) -> f32 {
        Vec3::from(self).dot(rhs.into())
    }

    /// `D3DXVec3Cross`.
    pub fn cross(self, rhs: Self) -> Self {
        Vec3::from(self).cross(rhs.into()).into()
    }

    /// `D3DXVec3Length`.
    pub fn length(self) -> f32 {
        Vec3::from(self).length()
    }

    /// `D3DXVec3LengthSq`.
    pub fn length_sq(self) -> f32 {
        Vec3::from(self).length_squared()
    }

    /// `D3DXVec3Normalize`. A zero-length vector normalizes to zero.
    pub fn normalize(self) -> Self {
        Vec3::from(self).normalize_or_zero().into()
    }

    /// `D3DXVec3Lerp`: `self + s * (rhs - self)`.
    pub fn lerp(self, rhs: Self, s: f32) -> Self {
        Vec3::from(self).lerp(rhs.into(), s).into()
    }

    /// `D3DXVec3Transform`: `(x, y, z, 1) * m`.
    pub fn transform(self, m: &RawMatrix44) -> RawVec4 {
        (Mat4::from(*m) * Vec3::from(self).extend(1.0)).into()
    }

    /// `D3DXVec3TransformCoord`: `(x, y, z, 1) * m`, divided by the resulting
    /// `w`.
    pub fn transform_coord(self, m: &RawMatrix44) -> Self {
        Mat4::from(*m).project_point3(self.into()).into()
    }

    /// `D3DXVec3TransformNormal`: `(x, y, z, 0) * m`, ignoring translation.
    pub fn transform_normal(self, m: &RawMatrix44) -> Self {
        Mat4::from(*m).transform_vector3(self.into()).into()
    }
}

impl RawVec4 {
    /// `D3DXVec4Dot`.
    pub fn dot(self, rhs: Self) -> f32 {
        Vec4::from(self).dot(rhs.into())
    }

    /// `D3DXVec4Length`.
    pub fn length(self) -> f32 {
        Vec4::from(self).length()
    }

    /// `D3DXVec4Normalize`. A zero-length vector normalizes to zero.
    pub fn normalize(self) -> Self {
        Vec4::from(self).normalize_or_zero().into()
    }

    /// `D3DXVec4Transform`: `self * m`.
    pub fn transform(self, m: &RawMatrix44) -> Self {
        (Mat4::from(*m) * Vec4::from(self)).into()
    }
}

impl RawQuat {
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    /// `D3DXQuaternionRotationAxis`. The axis does not need to be normalized.
    pub fn rotation_axis(axis: RawVec3, angle: f32) -> Self {
        Quat::from_axis_angle(Vec3::from(axis).normalize(), angle).into()
    }

    /// `D3DXQuaternionRotationMatrix`, for a pure rotation matrix.
    pub fn rotation_matrix(m: &RawMatrix44) -> Self {
        Quat::from_mat4(&Mat4::from(*m)).into()
    }

    /// `D3DXQuaternionMultiply`: rotation `self` followed by `rhs`.
    ///
    /// D3DX defines this as the Hamilton product `rhs * self`, so the operand
    /// order is the reverse of glam's `*`.
    pub fn multiply(self, rhs: Self) -> Self {
        (Quat::from(rhs) * Quat::from(self)).into()
    }

    /// `D3DXQuaternionDot`.
    pub fn dot(self, rhs: Self) -> f32 {
        Quat::from(self).dot(rhs.into())
    }

    /// `D3DXQuaternionLength`.
    pub fn length(self) -> f32 {
        Quat::from(self).length()
    }

    /// `D3DXQuaternionNormalize`.
    pub fn normalize(self) -> Self {
        Quat::from(self).normalize().into()
    }

    /// `D3DXQuaternionConjugate`.
    pub fn conjugate(self) -> Self {
        Quat::from(self).conjugate().into()
    }

    /// `D3DXQuaternionInverse`: the conjugate divided by the squared length.
    pub fn inverse(self) -> Self {
        let q = Quat::from(self);
        Quat::from_vec4(Vec4::from(q.conjugate()) / q.length_squared()).into()
    }

    /// `D3DXQuaternionSlerp`.
    pub fn slerp(self, rhs: Self, t: f32) -> Self {
        Quat::from(self).slerp(rhs.into(), t).into()
    }
}

impl RawMatrix44 {
    pub const IDENTITY: Self = Self {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    /// `D3DXMatrixMultiply`: `self * rhs`, so `self` is applied first.
    pub fn multiply(&self, rhs: &Self) -> Self {
        (Mat4::from(*rhs) * Mat4::from(*self)).into()
    }

    /// `D3DXMatrixTranspose`.
    pub fn transpose(&self) -> Self {
        Mat4::from(*self).transpose().into()
    }

    /// `D3DXMatrixDeterminant`.
    pub fn determinant(&self) -> f32 {
        Mat4::from(*self).determinant()
    }

    /// `D3DXMatrixInverse`. Returns `None` where D3DX returns null, i.e. for a
    /// zero determinant.
    pub fn inverse(&self) -> Option<Self> {
        let m = Mat4::from(*self);
        (m.determinant() != 0.0).then(|| m.inverse().into())
    }

    /// `D3DXMatrixTranslation`.
    pub fn translation(x: f32, y: f32, z: f32) -> Self {
        Mat4::from_translation(Vec3::new(x, y, z)).into()
    }

    /// `D3DXMatrixScaling`.
    pub fn scaling(x: f32, y: f32, z: f32) -> Self {
        Mat4::from_scale(Vec3::new(x, y, z)).into()
    }

    /// `D3DXMatrixRotationX`.
    pub fn rotation_x(angle: f32) -> Self {
        Mat4::from_rotation_x(angle).into()
    }

    /// `D3DXMatrixRotationY`.
    pub fn rotation_y(angle: f32) -> Self {
        Mat4::from_rotation_y(angle).into()
    }

    /// `D3DXMatrixRotationZ`.
    pub fn rotation_z(angle: f32) -> Self {
        Mat4::from_rotation_z(angle).into()
    }

    /// `D3DXMatrixRotationAxis`. The axis does not need to be normalized.
    pub fn rotation_axis(axis: RawVec3, angle: f32) -> Self {
        Mat4::from_axis_angle(Vec3::from(axis).normalize(), angle).into()
    }

    /// `D3DXMatrixRotationQuaternion`.
    pub fn rotation_quaternion(q: RawQuat) -> Self {
        Mat4::from_quat(q.into()).into()
    }

    /// `D3DXMatrixRotationYawPitchRoll`: roll about Z, then pitch about X,
    /// then yaw about Y.
    pub fn rotation_yaw_pitch_roll(yaw: f32, pitch: f32, roll: f32) -> Self {
        Self::rotation_z(roll)
            .multiply(&Self::rotation_x(pitch))
            .multiply(&Self::rotation_y(yaw))
    }

    /// `D3DXMatrixLookAtLH`.
    pub fn look_at_lh(eye: RawVec3, at: RawVec3, up: RawVec3) -> Self {
        Mat4::look_at_lh(eye.into(), at.into(), up.into()).into()
    }

    /// `D3DXMatrixPerspectiveFovLH`: depth maps to `[0, 1]`.
    pub fn perspective_fov_lh(fov_y: f32, aspect: f32, z_near: f32, z_far: f32) -> Self {
        Mat4::perspective_lh(fov_y, aspect, z_near, z_far).into()
    }

    /// `D3DXMatrixOrthoLH`: a view volume `width` by `height` centred on the
    /// origin, depth mapping to `[0, 1]`.
    pub fn ortho_lh(width: f32, height: f32, z_near: f32, z_far: f32) -> Self {
        let (w, h) = (width * 0.5, height * 0.5);
        Mat4::orthographic_lh(-w, w, -h, h, z_near, z_far).into()
    }
}

#[cfg(test)]
mod tests {
    use core::f32::consts::FRAC_PI_2;

    use super::*;

    const EPS: f32 = 1e-5;

    fn v3(x: f32, y: f32, z: f32) -> RawVec3 {
        RawVec3 { x, y, z }
    }

    fn assert_v3(actual: RawVec3, expected: RawVec3) {
        assert!(
            Vec3::from(actual).abs_diff_eq(expected.into(), EPS),
            "{actual:?} != {expected:?}"
        );
    }

    fn assert_m(actual: &RawMatrix44, expected: &RawMatrix44) {
        assert!(
            Mat4::from(*actual).abs_diff_eq(Mat4::from(*expected), EPS),
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn rotation_x_matches_d3dx_rows() {
        // D3DX: rows (1,0,0) (0,c,s) (0,-s,c), so +Y turns towards +Z.
        let m = RawMatrix44::rotation_x(FRAC_PI_2);
        assert_v3(v3(0.0, 1.0, 0.0).transform_normal(&m), v3(0.0, 0.0, 1.0));
        assert!((m.m[1][2] - 1.0).abs() < EPS && (m.m[2][1] + 1.0).abs() < EPS);
    }

    #[test]
    fn rotation_y_and_z_follow_left_handed_rows() {
        // RotationY rows (c,0,-s) (0,1,0) (s,0,c): +Z turns towards +X.
        let y = RawMatrix44::rotation_y(FRAC_PI_2);
        assert_v3(v3(0.0, 0.0, 1.0).transform_normal(&y), v3(1.0, 0.0, 0.0));
        // RotationZ rows (c,s,0) (-s,c,0) (0,0,1): +X turns towards +Y.
        let z = RawMatrix44::rotation_z(FRAC_PI_2);
        assert_v3(v3(1.0, 0.0, 0.0).transform_normal(&z), v3(0.0, 1.0, 0.0));
    }

    #[test]
    fn multiply_applies_left_operand_first() {
        let m =
            RawMatrix44::translation(1.0, 0.0, 0.0).multiply(&RawMatrix44::scaling(2.0, 2.0, 2.0));
        assert_v3(RawVec3::default().transform_coord(&m), v3(2.0, 0.0, 0.0));
    }

    #[test]
    fn transform_normal_ignores_translation() {
        let m = RawMatrix44::translation(5.0, 6.0, 7.0);
        assert_v3(v3(1.0, 2.0, 3.0).transform_normal(&m), v3(1.0, 2.0, 3.0));
        assert_v3(v3(1.0, 2.0, 3.0).transform_coord(&m), v3(6.0, 8.0, 10.0));
    }

    #[test]
    fn inverse_round_trips_and_rejects_singular() {
        let m = RawMatrix44::rotation_yaw_pitch_roll(0.3, -0.7, 1.1)
            .multiply(&RawMatrix44::translation(1.0, 2.0, 3.0));
        let inv = m.inverse().unwrap();
        assert_m(&m.multiply(&inv), &RawMatrix44::IDENTITY);
        assert_eq!(RawMatrix44::scaling(1.0, 0.0, 1.0).inverse(), None);
    }

    #[test]
    fn yaw_pitch_roll_single_axes() {
        assert_m(
            &RawMatrix44::rotation_yaw_pitch_roll(0.4, 0.0, 0.0),
            &RawMatrix44::rotation_y(0.4),
        );
        assert_m(
            &RawMatrix44::rotation_yaw_pitch_roll(0.0, 0.4, 0.0),
            &RawMatrix44::rotation_x(0.4),
        );
        assert_m(
            &RawMatrix44::rotation_yaw_pitch_roll(0.0, 0.0, 0.4),
            &RawMatrix44::rotation_z(0.4),
        );
    }

    #[test]
    fn axis_rotation_agrees_with_quaternion_and_unnormalized_axis() {
        let axis = v3(1.0, 2.0, -2.0);
        let q = RawQuat::rotation_axis(axis, 0.9);
        assert_m(
            &RawMatrix44::rotation_quaternion(q),
            &RawMatrix44::rotation_axis(axis, 0.9),
        );
        assert_m(
            &RawMatrix44::rotation_axis(v3(3.0, 0.0, 0.0), 0.9),
            &RawMatrix44::rotation_x(0.9),
        );
    }

    #[test]
    fn quaternion_multiply_applies_left_operand_first() {
        let a = RawQuat::rotation_axis(v3(1.0, 0.0, 0.0), 0.5);
        let b = RawQuat::rotation_axis(v3(0.0, 1.0, 0.0), 1.2);
        assert_m(
            &RawMatrix44::rotation_quaternion(a.multiply(b)),
            &RawMatrix44::rotation_quaternion(a).multiply(&RawMatrix44::rotation_quaternion(b)),
        );
    }

    #[test]
    fn quaternion_inverse_handles_unnormalized() {
        let q = RawQuat {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            w: 4.0,
        };
        let id = q.multiply(q.inverse());
        assert!(Quat::from(id).abs_diff_eq(Quat::IDENTITY, EPS), "{id:?}");
    }

    #[test]
    fn quaternion_matrix_round_trip() {
        let q = RawQuat::rotation_axis(v3(0.0, 1.0, 1.0), 2.0);
        let back = RawQuat::rotation_matrix(&RawMatrix44::rotation_quaternion(q));
        // q and -q are the same rotation.
        assert!(q.dot(back).abs() > 1.0 - EPS, "{q:?} vs {back:?}");
    }

    #[test]
    fn look_at_lh_puts_target_down_positive_z() {
        let view =
            RawMatrix44::look_at_lh(v3(0.0, 0.0, -5.0), RawVec3::default(), v3(0.0, 1.0, 0.0));
        assert_v3(RawVec3::default().transform_coord(&view), v3(0.0, 0.0, 5.0));
    }

    #[test]
    fn perspective_maps_near_and_far_to_unit_depth() {
        let proj = RawMatrix44::perspective_fov_lh(FRAC_PI_2, 1.0, 1.0, 100.0);
        assert!(v3(0.0, 0.0, 1.0).transform_coord(&proj).z.abs() < EPS);
        assert!((v3(0.0, 0.0, 100.0).transform_coord(&proj).z - 1.0).abs() < EPS);
        // D3DX stores w = z in _34, so the matrix is not affine.
        assert_eq!(proj.m[2][3], 1.0);
    }

    #[test]
    fn ortho_maps_volume_to_clip_space() {
        let proj = RawMatrix44::ortho_lh(4.0, 2.0, 0.0, 10.0);
        assert_v3(v3(2.0, 1.0, 10.0).transform_coord(&proj), v3(1.0, 1.0, 1.0));
        assert_v3(
            v3(-2.0, -1.0, 0.0).transform_coord(&proj),
            v3(-1.0, -1.0, 0.0),
        );
    }

    #[test]
    fn vector_basics() {
        assert_v3(
            v3(1.0, 0.0, 0.0).cross(v3(0.0, 1.0, 0.0)),
            v3(0.0, 0.0, 1.0),
        );
        assert_eq!(v3(3.0, 4.0, 0.0).length(), 5.0);
        assert_v3(RawVec3::default().normalize(), RawVec3::default());
        assert_v3(
            v3(0.0, 2.0, 4.0).lerp(v3(2.0, 4.0, 8.0), 0.5),
            v3(1.0, 3.0, 6.0),
        );
        assert_eq!(RawVec2 { x: 0.0, y: 0.0 }.normalize(), RawVec2::default());
    }
}
