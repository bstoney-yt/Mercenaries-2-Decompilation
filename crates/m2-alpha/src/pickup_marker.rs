//! The engine's markers over pickups: over each pickup lying near her, its
//! icon (its template's `TextureModel`, `pickup_ammo_rockets`) on the
//! pickups' backing (`pickup_background`), fading out at the edge of its
//! reach from the camera.
//!
//! From the 360 code: the HUD's per-frame update (823751B8) queues one for
//! each pickup not carried within 100 m of a player (82378890, which reads
//! the icon off the pickup's first `Pickup` record and its point off its
//! model's bound), and the simple-icon renderer (82375590) draws those
//! within the reach, measured from the view's camera, white, at an alpha
//! that is 1 out to 80% of the reach squared and then falls linearly in
//! the squared distance to 0 at the reach. It draws two squares, with `s`
//! = 0.75 × size × the view's scale `k`: the backing, centred `s` over
//! the point, of half-size `s × k`; and the icon, centred size + `s`/2
//! over the point, of half-size (size − 0.40625 `s`) × `k`. Ammunition,
//! cash and fuel measure their distance over the ground only.
//!
//! Inferred: `k` (a view field, +0x34) is 1.5, the 360's 720 lines over
//! the HUD's 480, and the picture is scaled from the 360's 720 lines to
//! the window's; with it the icon sits in the backing's box over its tab,
//! as the 360 footage shows (round 1 of the footage thread, 00:05:13).
//! The squares are taken as square (the renderer's width also takes a
//! factor, +0x2BD0, not identified). The marker shown for camera mode 4
//! (100 m) is not drawn: that mode isn't identified.

use egui::{Pos2, Rect, pos2, vec2};
use m2_math::glam::Vec3;

/// The texture under every pickup's icon. From the 360 code: the renderer
/// draws hash a7678ca4 under each.
pub const BACKGROUND: &str = "pickup_background";

/// Pickups within this far of her (metres, whole distance) have their
/// markers put up (from the 360 code: 82378890's test, 100 m squared).
pub const NEAR_HER: f32 = 100.0;

/// The share of the reach squared within which a marker is opaque.
const OPAQUE: f32 = 0.8;
/// The backing's half-size, of the size.
const BACKING: f32 = 0.75;
/// The icon's lift over size, and its shrink, of the backing's half-size.
const LIFT: f32 = 0.5;
const SHRINK: f32 = 0.40625;

/// A pickup's marker: the point it stands on (the top of the pickup), its
/// icon by name hash, and whether it fades by the distance over the
/// ground only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickupMarker {
    pub at: Vec3,
    pub icon: u32,
    pub flat: bool,
}

impl PickupMarker {
    /// How opaque it shows to a camera at `eye` with the markers' `reach`
    /// (metres), 0 to 1: gone at the reach and beyond, opaque within 80%
    /// of its square, linear in the squared distance between.
    pub fn alpha_from(&self, eye: Vec3, reach: f32) -> f32 {
        let mut off = self.at - eye;
        if self.flat {
            off.y = 0.0;
        }
        let d2 = off.length_squared();
        let r2 = reach * reach;
        if d2 >= r2 {
            0.0
        } else if d2 <= r2 * OPAQUE {
            1.0
        } else {
            (r2 - d2) / (r2 * (1.0 - OPAQUE))
        }
    }
}

/// The view's scale on the 360 (inferred: its 720 lines over the HUD's
/// 480), which the renderer applies once to the backing's lift and twice
/// to its half-size, and to the icon's half-size, but not to the size the
/// icon is lifted by.
const VIEW_SCALE: f32 = 1.5;

/// The squares a marker of `size` is drawn in, its point on the screen at
/// `p` and `unit` screen points to a pixel of the 360's 1280x720 picture:
/// the backing's, then the icon's.
pub fn squares(p: Pos2, size: f32, unit: f32) -> (Rect, Rect) {
    let k = VIEW_SCALE;
    let backing = BACKING * size * k;
    let icon = (size - SHRINK * backing) * k;
    (
        Rect::from_center_size(
            pos2(p.x, p.y - backing * unit),
            vec2(2.0, 2.0) * backing * k * unit,
        ),
        Rect::from_center_size(
            pos2(p.x, p.y - (size + LIFT * backing) * unit),
            vec2(2.0, 2.0) * icon * unit,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backing_is_the_hash_the_renderer_draws() {
        assert_eq!(m2_assets::name_hash(BACKGROUND), 0xa767_8ca4);
    }

    #[test]
    fn a_marker_fades_over_the_last_fifth_of_its_reach_squared() {
        let m = PickupMarker {
            at: Vec3::ZERO,
            icon: 1,
            flat: false,
        };
        let at = |d: f32| m.alpha_from(Vec3::new(d, 0.0, 0.0), 20.0);
        assert_eq!(at(3.0), 1.0);
        // Opaque to sqrt(0.8) × 20 = 17.9 m.
        assert_eq!(at(17.88), 1.0);
        // Half at d² = 0.9 × 400.
        assert!((at(360f32.sqrt()) - 0.5).abs() < 1e-4);
        assert_eq!(at(20.0), 0.0);
        assert_eq!(at(25.0), 0.0);
        // Height counts unless it is flat.
        let up = Vec3::new(0.0, 19.0, 0.0);
        assert!(m.alpha_from(up, 20.0) < 1.0);
        let flat = PickupMarker { flat: true, ..m };
        assert_eq!(flat.alpha_from(up, 20.0), 1.0);
    }

    #[test]
    fn the_icon_sits_in_the_backing_over_its_tab() {
        // At 720 lines, size 12: the backing's half-size 0.75 × 12 × 1.5
        // = 13.5, scaled again: 20.25, lifted 13.5; its tab hangs below.
        let (backing, icon) = squares(pos2(100.0, 200.0), 12.0, 1.0);
        assert_eq!(backing.center(), pos2(100.0, 186.5));
        assert_eq!(backing.size(), vec2(40.5, 40.5));
        // The icon: (12 − 0.40625 × 13.5) × 1.5 = 9.77 half-size, centred
        // 12 + 6.75 up, inside the backing's upper part.
        assert_eq!(icon.center(), pos2(100.0, 181.25));
        assert!((icon.width() - 19.546875).abs() < 1e-4);
        assert!(backing.contains_rect(icon));
        assert!(icon.center().y < backing.center().y);
        // At 1080 lines all of it half as large again.
        let (big, _) = squares(pos2(100.0, 200.0), 12.0, 1.5);
        assert_eq!(big.size(), backing.size() * 1.5);
    }
}
