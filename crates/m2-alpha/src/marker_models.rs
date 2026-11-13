//! The scripts' markers the game draws as models in the world: a race's
//! tripwire across a gate (`Marker.AddTripwire`), its rings and finish
//! (`Marker.Add3D`: `global_airring`, `global_tripwirefinish`), and the
//! mark held over someone being delivered (`Marker.Add`, [`Pin`]).
//!
//! # From the 360 code
//!
//! All three make one kind of marker record (8231B230 by 82381970;
//! 8231B5A8 and 8231B828 by 823815D8), and the marker update (82382578)
//! gives each a model once its model has loaded, coloured the record's
//! colour (the call's 0..255 values over 255), and places it:
//!
//! - A tripwire draws `global_tripwire` (the record's model,
//!   `0xC2DC97E1`), at its point, turned its yaw (degrees over the vertical
//!   axis) and unscaled. Its two end nodes (`0xAB814FE1` and `0x98EF01C8`,
//!   1 m either side of the middle) are moved out along the model's x to
//!   half the width either side, so the wire spans the width (held to
//!   1..50 m, [`m2_game::hud::TRIPWIRE_WIDTH`]).
//! - `Marker.Add3D` draws its model where its target stands when it is
//!   called, turned as the target faces (`Object.GetYaw`'s angle) and
//!   scaled by its scale.
//! - `Marker.AddDisc` (`_MarkerAddDisc`, 8231B410 into 823817E0) draws
//!   `global_locationmarker` (`0xF84AEB3C`, a unit disc: radius 1, 1 m
//!   high) at its target's place (or its point) lifted by its lift,
//!   scaled by its radius; its arguments are `(target, radius, r, g, b,
//!   lift)` with radius 1, colour 255 each and lift 0 where left out, the
//!   colour over 255 (0x820CB628) as the others'.
//! - `Marker.Add` draws `global_objectivemarker` (`0x40FF196A`) at its
//!   target's place (followed as it moves) plus its offset, scaled by its
//!   scale, and lifted by the target's height (its bounds' top over their
//!   bottom). Without a target the offset is its place in the world. Its
//!   turn is a quarter turn over the vertical axis.
//!
//! The turn replaces the model's: a quarter turn for a marker without a
//! yaw, its yaw for one with (the record's flag 0x80; 82855CE8 sets the
//! rotation, it does not add to it).
//!
//! # Not established
//!
//! - How the colour is applied in the game's shaders: here it multiplies
//!   the surface's colour ([`m2_render::DrawCall::tint`]), inferred.
//! - Whether the models fade with distance as the icon markers do (the
//!   records carry 200 m and 300 m as `Marker.Add`'s icons do): they are
//!   drawn at any distance here.
//! - A disc's scale: its one scale is the radius, applied to its height as
//!   well here (the model's top and bottom nodes are left where they are);
//!   and whether it follows its target (the 360 takes the target's place
//!   when it is called; here it follows, as a pin does).
//! - A pin's lift by its target's height: the scripts' world here has no
//!   bounds for its objects, so the pin stands its offset over the
//!   target's place only.

use m2_math::glam::{Mat4, Quat, Vec3};

/// `global_tripwire`'s name hash: the model a tripwire draws.
pub const TRIPWIRE_MODEL: u32 = 0xC2DC_97E1;
/// `global_objectivemarker`'s name hash: the model a pin draws.
pub const PIN_MODEL: u32 = 0x40FF_196A;
/// `global_locationmarker`'s name hash: the model a disc draws.
pub const DISC_MODEL: u32 = 0xF84A_EB3C;
/// A tripwire model's end nodes: the one 1 m to the model's -x side and
/// the one 1 m to its +x side.
pub const TRIPWIRE_ENDS: [u32; 2] = [0xAB81_4FE1, 0x98EF_01C8];
/// The turn a marker without a yaw takes (radians about the vertical).
const QUARTER_TURN: f32 = std::f32::consts::FRAC_PI_2;

/// One marker model to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerModel {
    /// The model's name hash.
    pub model: u32,
    pub at: Vec3,
    /// Radians about the vertical; `None` for the quarter turn.
    pub yaw: Option<f32>,
    pub scale: f32,
    /// A tripwire's span, metres.
    pub width: Option<f32>,
    /// The scripts' colour, 0..255.
    pub colour: [u8; 3],
}

impl MarkerModel {
    /// Model to world.
    pub fn world(&self) -> Mat4 {
        let turn = Quat::from_rotation_y(self.yaw.unwrap_or(QUARTER_TURN));
        Mat4::from_scale_rotation_translation(Vec3::splat(self.scale), turn, self.at)
    }

    /// The tint its colour gives ([`m2_render::DrawCall::tint`]).
    pub fn tint(&self) -> [f32; 4] {
        let [r, g, b] = self.colour.map(|c| f32::from(c) / 255.0);
        [r, g, b, 1.0]
    }

    /// The model-space move of each of the model's nodes (by their name
    /// hashes, in order): a tripwire's ends out to half its width either
    /// side; nothing for any other node or marker.
    pub fn node_moves(&self, nodes: &[u32]) -> Vec<Vec3> {
        let out = self.width.map_or(0.0, |w| w * 0.5 - 1.0);
        nodes
            .iter()
            .map(|&h| match h {
                _ if self.width.is_none() => Vec3::ZERO,
                h if h == TRIPWIRE_ENDS[0] => Vec3::new(-out, 0.0, 0.0),
                h if h == TRIPWIRE_ENDS[1] => Vec3::new(out, 0.0, 0.0),
                _ => Vec3::ZERO,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_hashes_are_the_names() {
        assert_eq!(m2_assets::name_hash("global_tripwire"), TRIPWIRE_MODEL);
        assert_eq!(m2_assets::name_hash("global_objectivemarker"), PIN_MODEL);
        assert_eq!(m2_assets::name_hash("global_locationmarker"), DISC_MODEL);
    }

    /// A disc's unit model is scaled to its radius where it is lifted to,
    /// in its colour.
    #[test]
    fn a_disc_is_its_radius_across_at_its_lift() {
        let disc = MarkerModel {
            model: DISC_MODEL,
            at: Vec3::new(3.0, 1.0 + 0.05, -2.0),
            yaw: None,
            scale: 4.0,
            width: None,
            colour: [0, 255, 0],
        };
        let world = disc.world();
        let centre = world.transform_point3(Vec3::ZERO);
        assert!(centre.distance(Vec3::new(3.0, 1.05, -2.0)) < 1e-5);
        // The rim (radius 1 in the model) lies 4 m out, flat.
        let rim = world.transform_point3(Vec3::X);
        assert!((rim.with_y(0.0).distance(centre.with_y(0.0)) - 4.0).abs() < 1e-4);
        assert!((rim.y - centre.y).abs() < 1e-5);
        assert_eq!(disc.tint(), [0.0, 1.0, 0.0, 1.0]);
        assert!(
            disc.node_moves(&[DISC_MODEL])
                .iter()
                .all(|m| *m == Vec3::ZERO)
        );
    }

    #[test]
    fn a_tripwire_spans_its_width() {
        let wire = MarkerModel {
            model: TRIPWIRE_MODEL,
            at: Vec3::new(10.0, 0.0, 5.0),
            yaw: Some(0.0),
            scale: 1.0,
            width: Some(20.0),
            colour: [255, 0, 0],
        };
        let moves = wire.node_moves(&[TRIPWIRE_MODEL, TRIPWIRE_ENDS[1], TRIPWIRE_ENDS[0]]);
        // The ends sit at x = +-1 in the model: moved to +-10.
        assert_eq!(moves[0], Vec3::ZERO);
        assert_eq!(Vec3::X + moves[1], Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(-Vec3::X + moves[2], Vec3::new(-10.0, 0.0, 0.0));
        assert_eq!(wire.tint(), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn markers_without_a_yaw_take_a_quarter_turn() {
        let pin = MarkerModel {
            model: PIN_MODEL,
            at: Vec3::ZERO,
            yaw: None,
            scale: 0.25,
            width: None,
            colour: [255; 3],
        };
        let x = pin.world().transform_vector3(Vec3::X);
        assert!((x - Vec3::new(0.0, 0.0, -0.25)).length() < 1e-5, "{x}");
        assert_eq!(pin.node_moves(&[TRIPWIRE_ENDS[0]]), vec![Vec3::ZERO]);
    }
}
