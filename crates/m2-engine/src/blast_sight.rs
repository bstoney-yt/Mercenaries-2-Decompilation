//! Whether an area blast sees what it would hurt: the 360's line of sight
//! for a blast's damage (from the 360 code, 0x82291B88 and 0x82292778).
//!
//! An area-damage object (made by 0x82291440) gathers what is in its
//! reach and, for each body that passes its type, angle and group tests,
//! either hurts it at once (0x82291640) or, by its `Explosive` record's
//! `Detail` (+0x20, [`Detail`]), first casts rays from its heart to it:
//!
//! - **Which bodies wait for rays:** those of type 0, 1, 2, 3, 4, 8 or 10
//!   (a prop, a person, a car, a tank, a helicopter, a jet, a boat) unless
//!   `Detail` is `Low` (2); the rest are hurt at once.
//! - **How many rays:** a person's go to bones (0x8241E538): with `High`
//!   (0) two, to [`LEFT_SHIN`] and [`UPPER_BODY`]; with `Medium` (1) one,
//!   to [`CHEST`]. With more than [`MANY`] bodies waiting, the object's
//!   `Detail` drops to the larger of one less and `Medium` (0x82293218):
//!   so `High` casts one ray to each.
//! - **What a ray sees:** [`MASK`], a layer mask the world's ray filter
//!   tests the hit body's layer against (0x827F8550,
//!   [`crate::collision_layers::seen_by`]): buildings, vehicles, props and
//!   boats, not people.
//! - **When:** the rays are queued in the physics world's batch
//!   (0x8215A0F8) and read by the object's next update (0x82292778): one
//!   frame later.
//! - **What it takes:** a ray is clear when it hit nothing, or hit the
//!   body it was cast at. With any clear, the body is hurt as one hurt at
//!   once (0x82291640), times [`clear_share`]: one of a pair clear is
//!   half of it, both or a lone ray all of it.
//!
//! From the 360 data: the `Detail` of the 65 distinct `Explosive` records
//! is `High` in 39, `Medium` in 12 and `Low` in 14; a car's death blasts
//! (`fx_Explosion_Large`, `Explosion (RPG Frag)`) are `High`. The schema's
//! default (0x823EF9F0) is `High`.

/// What a blast's rays see (0x82291B88's rays carry the word at
/// 0x82DE2F24): layers 3, 4, 6, 7 (buildings), 8 to 11 (vehicles), 12,
/// 13, 15, 16 (props), 19 and 29 (a boat's), and layer 0 as every query
/// sees.
pub const MASK: u32 = 0x2009_bfd8;

/// More bodies than this waiting for rays, and each gets fewer
/// (0x82291B88 -> 0x82293218).
pub const MANY: usize = 8;

/// The bones a person's rays go to (name hashes): `Bone_LShin`, the
/// upper body's node (as [`crate::death::UPPER_NODE`]) and the chest.
pub const LEFT_SHIN: u32 = 0xa76c_9842;
pub const UPPER_BODY: u32 = 0x705c_4508;
pub const CHEST: u32 = 0x4c77_33ed;

/// An `Explosive` record's `Detail` (`ExplosiveDetailEnum`, 0x823DC2B0:
/// `High`, then two more by index; their names are not read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// 0: two rays a person.
    High,
    /// 1: one ray a person.
    Medium,
    /// 2: no rays: hurt at once.
    Low,
}

impl Detail {
    /// The record's word: 0, 1 or 2 (anything else as `Low`, which the
    /// 360's tests treat as neither of the others).
    pub fn from_word(w: u32) -> Self {
        match w {
            0 => Self::High,
            1 => Self::Medium,
            _ => Self::Low,
        }
    }

    fn word(self) -> i32 {
        match self {
            Self::High => 0,
            Self::Medium => 1,
            Self::Low => 2,
        }
    }

    /// With `waiting` bodies waiting for rays: more than [`MANY`] and it
    /// drops to the larger of one less and `Medium` (0x82293218).
    pub fn for_count(self, waiting: usize) -> Self {
        if waiting > MANY {
            Self::from_word((self.word() - 1).max(1) as u32)
        } else {
            self
        }
    }

    /// Whether a body of a type that may wait for rays does (`Low` hurts
    /// it at once).
    pub fn casts(self) -> bool {
        self != Self::Low
    }

    /// The bones a person's rays go to.
    pub fn person_bones(self) -> &'static [u32] {
        match self {
            Self::High => &[LEFT_SHIN, UPPER_BODY],
            Self::Medium => &[CHEST],
            Self::Low => &[],
        }
    }
}

/// Each template's blast's [`Detail`], by template: those with
/// `Explosive` (a record without the field takes the schema's `High`).
pub fn details(db: &m2_assets::layer::Layer<'_>) -> std::collections::HashMap<u32, Detail> {
    let t = crate::tuning::Table::new(db, "Explosive");
    let field = m2_assets::name_hash("Detail");
    t.templates()
        .filter_map(|id| {
            let r = t.of_template(id).first()?;
            Some((id, t.word(r, field).map_or(Detail::High, Detail::from_word)))
        })
        .collect()
}

/// How far above its heart the second ray of a pair starts, metres
/// (0x82291B88: its heart's height plus 0x820D9EDC, 0.5); the first, and
/// a lone ray, start at its heart.
pub const SECOND_RAY_RISE: f32 = 0.5;

/// Where each of `n` rays starts from a blast's heart `at` (the second
/// of a pair [`SECOND_RAY_RISE`] up).
pub fn ray_starts(at: m2_math::glam::Vec3, n: usize) -> Vec<m2_math::glam::Vec3> {
    (0..n)
        .map(|k| {
            if k == 1 {
                at + m2_math::glam::Vec3::Y * SECOND_RAY_RISE
            } else {
                at
            }
        })
        .collect()
}

/// What a body whose rays came back `clear` takes of the blast: with
/// none clear, none; of a pair (`High`), half for each clear; a lone ray
/// clear, all (0x82292778: the clear rays counted, halved for a pair).
pub fn clear_share(clear: &[bool]) -> f32 {
    let n = clear.iter().filter(|&&c| c).count() as f32;
    if clear.len() >= 2 { n * 0.5 } else { n }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::name_hash;

    #[test]
    fn its_bones_are_named_as_the_rig_names_them() {
        assert_eq!(name_hash("Bone_LShin"), LEFT_SHIN);
        assert_eq!(UPPER_BODY, crate::death::UPPER_NODE);
        assert_eq!(name_hash("Detail"), 0x3067_b31a);
    }

    #[test]
    fn a_pair_halves_and_a_lone_ray_is_all_or_nothing() {
        assert_eq!(clear_share(&[true, true]), 1.0);
        assert_eq!(clear_share(&[true, false]), 0.5);
        assert_eq!(clear_share(&[false, true]), 0.5);
        assert_eq!(clear_share(&[false, false]), 0.0);
        assert_eq!(clear_share(&[true]), 1.0);
        assert_eq!(clear_share(&[false]), 0.0);
    }

    #[test]
    fn the_second_of_a_pair_starts_half_a_metre_up() {
        use m2_math::glam::Vec3;
        let at = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(ray_starts(at, 1), vec![at]);
        assert_eq!(ray_starts(at, 2), vec![at, Vec3::new(1.0, 2.5, 3.0)]);
    }

    #[test]
    fn many_waiting_cast_fewer_rays() {
        assert_eq!(Detail::High.for_count(8), Detail::High);
        assert_eq!(Detail::High.for_count(9), Detail::Medium);
        assert_eq!(Detail::Medium.for_count(9), Detail::Medium);
        assert_eq!(Detail::Low.for_count(9), Detail::Medium);
        assert_eq!(Detail::High.person_bones().len(), 2);
        assert_eq!(Detail::Medium.person_bones(), &[CHEST]);
        assert!(!Detail::Low.casts());
    }

    #[test]
    fn its_mask_sees_vehicles_and_buildings_but_not_people() {
        use crate::collision_layers::seen_by;
        for layer in [3, 4, 6, 7, 8, 9, 11, 12, 13, 15, 16, 29] {
            assert!(seen_by(MASK, layer), "{layer}");
        }
        for layer in [2, 5, 14, 17, 18, 30] {
            assert!(!seen_by(MASK, layer), "{layer}");
        }
    }
}
