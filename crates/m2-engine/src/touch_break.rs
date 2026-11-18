//! Objects a character's touch destroys: the 360's break-on-touch piles.
//!
//! **From the 360 code:**
//! - When an object is placed (0x821831B0), 0x82183ED0 looks it up in
//!   the per-object attribute store 0x83653C4C under key
//!   [`BREAK_ON_TOUCH_LABEL`] (0x5664EE83); if it has it, every body of
//!   the object gets body property 0x200A.
//! - A character proxy's contact callback (0x82176830) that touches a body
//!   with 0x200A calls the touched object's vfunc +0xB4 (0x821859E0) and
//!   posts the removal of 0x200A (queue 0x8322DC38), so it fires once;
//!   but only when the toucher's human object has its byte +0x40 set
//!   (vfunc +0x30, 0x8217DC30), "riding with a player", which walking
//!   people never have (`m2_alpha`'s `touch_break` has the gate), and
//!   never for a body pushed as a dynamic prop (property 0x2009).
//! - 0x821859E0 posts a damage event of 1,000,000 (0x820F9604) on the
//!   touched object itself through the ordinary damage queue
//!   (0x8318C480), unless the object is in `PhysicsPropertyUncrushable`
//!   (table 0x83651B30) or is dead or destroyed (its byte +0x15, set by
//!   0x821831B0 when its health is under 0.01). The toucher takes nothing.
//!
//! **From samples:** the key is a `Label` value. Ten templates carry it,
//! `_global_trashpile01` to `03` and `_global_pile01` to `07`, placed
//! 4,588 times; `PhysicsPropertyUncrushable` is on `C4` and
//! `C4 Projectile` only.
//!
//! **Inferred:** that a touch is the character's capsule meeting the
//! object's box ([`capsule_meets_box`]); the 360 fires on any contact the
//! proxy reports, with no speed threshold, and its proxy meets the
//! object's collision shapes, not a box.

use std::collections::HashSet;

use m2_assets::Layer;
use m2_math::glam::{Mat4, Vec3};

use crate::tuning::Table;

/// The `Label` value that makes an object break when a character touches
/// it (0x82183ED0).
pub const BREAK_ON_TOUCH_LABEL: u32 = 0x5664_ee83;

/// `Label`'s value field (`m2_game`'s `LABEL_FIELD`).
const LABEL_FIELD: u32 = 0xfd08_4cce;

/// The damage a touch deals the object (0x821859E0, 0x820F9604).
pub const TOUCH_DAMAGE: f32 = 1_000_000.0;

/// The templates carrying [`BREAK_ON_TOUCH_LABEL`], by id.
pub fn touch_breakers(db: &Layer<'_>) -> HashSet<u32> {
    let t = Table::new(db, "Label");
    t.templates()
        .filter(|&id| {
            t.of_template(id)
                .iter()
                .any(|r| t.word(r, LABEL_FIELD) == Some(BREAK_ON_TOUCH_LABEL))
        })
        .collect()
}

/// The templates carrying `PhysicsPropertyUncrushable` (0x823A92E8,
/// table 0x83651B30), by id: a touch never breaks them.
pub fn uncrushable(db: &Layer<'_>) -> HashSet<u32> {
    Table::new(db, "PhysicsPropertyUncrushable")
        .templates()
        .collect()
}

/// Whether a character's capsule (`feet` at its bottom, `radius`, total
/// `height`) meets a box (`bounds`, model space) placed at `at`: its
/// axis, sampled at its two sphere centres and between, comes within
/// `radius` of the box.
pub fn capsule_meets_box(
    feet: Vec3,
    radius: f32,
    height: f32,
    at: Mat4,
    bounds: (Vec3, Vec3),
) -> bool {
    let to_box = at.inverse();
    let (lo, hi) = bounds;
    let bottom = feet + Vec3::Y * radius;
    let top = feet + Vec3::Y * (height - radius).max(radius);
    (0..=4).any(|k| {
        let p = bottom.lerp(top, k as f32 / 4.0);
        let local = to_box.transform_point3(p);
        let near = local.clamp(lo, hi);
        // The box may be scaled: measure in the world.
        at.transform_point3(near).distance_squared(p) <= radius * radius
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::name_hash;
    use m2_math::glam::Quat;

    #[test]
    fn the_label_is_the_360s_key() {
        assert_eq!(BREAK_ON_TOUCH_LABEL, 0x5664_ee83);
        assert_ne!(name_hash("Label"), 0);
    }

    #[test]
    fn a_capsule_touches_a_box_it_stands_against() {
        let pile = (Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.8, 1.0));
        let at =
            Mat4::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(5.0, 0.0, 5.0));
        let (r, h) = (0.35, 1.8);
        // Inside it, and just against its side.
        assert!(capsule_meets_box(Vec3::new(5.0, 0.0, 5.0), r, h, at, pile));
        let side = at.transform_point3(Vec3::new(1.0 + r - 0.01, 0.0, 0.0));
        assert!(capsule_meets_box(side, r, h, at, pile));
        // A step away.
        let off = at.transform_point3(Vec3::new(1.0 + r + 0.3, 0.0, 0.0));
        assert!(!capsule_meets_box(off, r, h, at, pile));
        // Standing on it counts; well above it does not.
        assert!(capsule_meets_box(Vec3::new(5.0, 0.8, 5.0), r, h, at, pile));
        assert!(!capsule_meets_box(Vec3::new(5.0, 2.0, 5.0), r, h, at, pile));
    }
}
