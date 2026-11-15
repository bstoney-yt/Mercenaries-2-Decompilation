//! How a part destroyed by damage goes down: as it always does, or burnt
//! (a tree blown up: its trunk catching fire, its leaves flying off as
//! embers).
//!
//! **From the 360 code.** When a part's health runs out (0x82289020, the
//! part damage step, called from 0x82289650), it posts one of two
//! messages to the object's part state machine (the object event queue
//! 0x83232DB0): [`DESTROY_MSG`] (`DestroyMsg`), or [`FIRE_DESTROY_MSG`]
//! (`0x3D0D4C99`, its name not in the dictionary). Which is chosen by the
//! damage's kind (a 3-bit field of the damage record) and its damage key
//! ([`destroy_message`]): kinds 0 and 3 always destroy; kind 1 burns for
//! the `Explosion` and `ExplosionLarge` keys and destroys for the rest;
//! the other kinds always burn.
//!
//! **From samples** (the trees' state machines, `global_env_palmtree01`):
//! the trunk's intact state (`0xACB51200`) goes to `CollapseState` on
//! `DestroyMsg` and to [`FIRE`] on [`FIRE_DESTROY_MSG`]. `CollapseState`
//! sets the trunk to `DestroyedState`; [`FIRE`] starts a fire on the
//! trunk's wreck and sets it to [`FIRE_DESTROYED`]. Each leaf's machine
//! has [`DEBRIS`] and [`FIRE_DEBRIS`] (the latter starting embers on the
//! leaf), both showing the same leaf. The tree scripts
//! (`OnStateChange`) burn the leaves' material when told of
//! [`FIRE_DEBRIS`] or [`FIRE_DESTROYED`].
//!
//! **Inferred:** that the 360's kind 1 is a blast's (its two burning keys
//! are the explosions'), so a blast with either key burns what it
//! destroys ([`blast_burns`]); a round or a crash destroys. Which game
//! events give kind 2 (always burning) is not traced.

pub use m2_assets::model::DESTROY_MSG;

/// The message that destroys a part burning (`0x3D0D4C99`, 0x82289020).
pub const FIRE_DESTROY_MSG: u32 = 0x3d0d_4c99;
/// The trunk's state on [`FIRE_DESTROY_MSG`] (`0x0753C8F7`, its name not
/// in the dictionary): its fire starts, then it sets [`FIRE_DESTROYED`].
pub const FIRE: u32 = 0x0753_c8f7;
/// `FireDestroyedState`: the trunk burnt down.
pub const FIRE_DESTROYED: u32 = 0x0046_41a5;
/// `FireDebrisState`: a leaf flying off burning.
pub const FIRE_DEBRIS: u32 = 0x4c88_281b;
/// `CollapseState`: the trunk coming down.
pub const COLLAPSE: u32 = 0x6946_83eb;
/// `DestroyedState`: the trunk down.
pub const DESTROYED: u32 = 0x7687_df41;
/// `DebrisState`: a leaf flying off.
pub const DEBRIS: u32 = 0x37a6_05ff;

/// The `Explosion` damage key.
pub const EXPLOSION: u32 = 0x1693_0afe;
/// The `ExplosionLarge` damage key.
pub const EXPLOSION_LARGE: u32 = 0xdc0a_895f;

/// The message a part destroyed by damage of `kind` (the 360's 3-bit
/// field) and damage key `key` takes (0x82289020).
pub fn destroy_message(kind: u32, key: u32) -> u32 {
    match kind {
        0 | 3 => DESTROY_MSG,
        1 if key != EXPLOSION && key != EXPLOSION_LARGE => DESTROY_MSG,
        _ => FIRE_DESTROY_MSG,
    }
}

/// Whether a blast of damage key `key` burns what it destroys (taken as
/// the 360's kind 1; inferred).
pub fn blast_burns(key: u32) -> bool {
    destroy_message(1, key) == FIRE_DESTROY_MSG
}

/// The states a destroyed tree's parts go through, in order, the trunk's
/// first, then its leaves': burning ([`FIRE`], [`FIRE_DESTROYED`],
/// [`FIRE_DEBRIS`]) or not ([`COLLAPSE`], [`DESTROYED`], [`DEBRIS`]).
pub fn tree_states(burning: bool) -> [u32; 3] {
    if burning {
        [FIRE, FIRE_DESTROYED, FIRE_DEBRIS]
    } else {
        [COLLAPSE, DESTROYED, DEBRIS]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::name_hash;

    #[test]
    fn the_states_are_named_by_their_hash() {
        assert_eq!(name_hash("FireDestroyedState"), FIRE_DESTROYED);
        assert_eq!(name_hash("FireDebrisState"), FIRE_DEBRIS);
        assert_eq!(name_hash("CollapseState"), COLLAPSE);
        assert_eq!(name_hash("DestroyedState"), DESTROYED);
        assert_eq!(name_hash("DebrisState"), DEBRIS);
        assert_eq!(name_hash("Explosion"), EXPLOSION);
        assert_eq!(name_hash("ExplosionLarge"), EXPLOSION_LARGE);
        assert_eq!(name_hash("DestroyMsg"), DESTROY_MSG);
    }

    /// 0x82289020's choice.
    #[test]
    fn the_kind_and_key_pick_the_message() {
        let other = name_hash("Bullet");
        for key in [EXPLOSION, EXPLOSION_LARGE, other] {
            assert_eq!(destroy_message(0, key), DESTROY_MSG);
            assert_eq!(destroy_message(3, key), DESTROY_MSG);
            assert_eq!(destroy_message(2, key), FIRE_DESTROY_MSG);
            assert_eq!(destroy_message(4, key), FIRE_DESTROY_MSG);
        }
        assert_eq!(destroy_message(1, EXPLOSION), FIRE_DESTROY_MSG);
        assert_eq!(destroy_message(1, EXPLOSION_LARGE), FIRE_DESTROY_MSG);
        assert_eq!(destroy_message(1, other), DESTROY_MSG);
        assert!(blast_burns(EXPLOSION) && blast_burns(EXPLOSION_LARGE));
        assert!(!blast_burns(other));
        assert_eq!(tree_states(true), [FIRE, FIRE_DESTROYED, FIRE_DEBRIS]);
        assert_eq!(tree_states(false), [COLLAPSE, DESTROYED, DEBRIS]);
    }
}
