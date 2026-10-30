//! The engine's markers over vehicles' entrances: while she is on foot,
//! over each way into a vehicle near her, an icon of the seat it gives
//! (the wheel's, the gunner's or a passenger's).
//!
//! From the 360 code: the HUD's per-frame update (823751B8) queues them
//! while the markers are on (`EnableVehicleEntranceMarkers`, on from the
//! start, 82374FB0) and no player rides anything (82376A48): for each
//! vehicle within 100 m of a player, each of its entrances with a
//! `UseDistance` above 0 within 10 m of a player that gives a seat (the
//! one 82299D68 picks: free, or one to hijack), at the entrance's point
//! 1 m up, with its seat's texture (82376F08): `global_vehiclemarker` for
//! the driver's, `global_vehiclemarker_gunner` for a gunner's, else
//! `global_vehiclemarker_passenger`. The simple-icon renderer (82375590)
//! draws those within the reach of the view's camera
//! (`SetVehicleEntranceMarkerVisibleDistance`, 10 m) as one square
//! centred the marker's size (`SetVehicleEntranceMarkerSize`, 16) over
//! the point, of half-size size Ã— the view's scale.
//!
//! Inferred, as for [`crate::pickup_marker`] (the same renderer): the
//! view's scale is 1.5 and the picture scaled from the 360's 720 lines;
//! the fade over the last fifth of the reach squared. Not drawn: the
//! variant the 360 gives a passenger's seat of an entrance flagged as a
//! cargo hold's (texture 3e073630, not among the game's textures).

use egui::{Pos2, Rect, pos2, vec2};
use m2_engine::seats::Role;
use m2_math::glam::Vec3;

/// Vehicles within this far of her have their entrances looked at
/// (metres; from the 360 code: 82376A48's test, 10000 squared).
pub const NEAR_VEHICLE: f32 = 100.0;
/// Entrances within this far of her are marked, and markers within this
/// far of the camera drawn (metres; the 360's default, 100 squared).
pub const REACH: f32 = 10.0;
/// How far over the entrance's point the marker stands (metres).
pub const LIFT: f32 = 1.0;
/// The marker's size, in the 360's pixels (its default, 0x10).
pub const SIZE: f32 = 16.0;
/// The view's scale (inferred, as the pickups' markers').
const VIEW_SCALE: f32 = 1.5;

/// The texture marking an entrance to a seat of `role`.
pub fn texture(role: Role) -> &'static str {
    match role {
        Role::Driver => "global_vehiclemarker",
        Role::Gunner => "global_vehiclemarker_gunner",
        Role::Passenger => "global_vehiclemarker_passenger",
    }
}

/// An entrance's marker: the point it stands on (1 m over the entrance)
/// and the seat it leads to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntranceMarker {
    pub at: Vec3,
    pub role: Role,
}

/// The square a marker is drawn in, its point on the screen at `p` and
/// `unit` screen points to a pixel of the 360's 1280x720 picture.
pub fn square(p: Pos2, unit: f32) -> Rect {
    Rect::from_center_size(
        pos2(p.x, p.y - SIZE * unit),
        vec2(2.0, 2.0) * SIZE * VIEW_SCALE * unit,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_textures_are_the_hashes_the_360_picks() {
        let h = |r| m2_assets::name_hash(texture(r));
        assert_eq!(h(Role::Driver), 0xb1d6_c73b);
        assert_eq!(h(Role::Gunner), 0xf2cb_0af7);
        assert_eq!(h(Role::Passenger), 0x3fdc_ee8a);
    }

    #[test]
    fn the_marker_sits_its_size_over_the_point() {
        // At 720 lines: centred 16 up, 48 across.
        let r = square(pos2(100.0, 200.0), 1.0);
        assert_eq!(r.center(), pos2(100.0, 184.0));
        assert_eq!(r.size(), vec2(48.0, 48.0));
        assert_eq!(square(pos2(100.0, 200.0), 1.5).size(), vec2(72.0, 72.0));
    }
}
