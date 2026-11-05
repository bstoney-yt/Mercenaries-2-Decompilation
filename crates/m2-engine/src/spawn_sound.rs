//! What a template sounds like as one of its objects comes into being: its
//! `SoundEffect` record. Every blast template (`Explosion (Grenade)`,
//! `Explosion (Airstike Bomb)`, ...) carries one, so a blast is heard as
//! the template it spawns says, not as a sound picked for it.
//!
//! The record's words, by offset: `SoundSet` at 0x0 (the `Sounds` table's
//! key, [`crate::gun_sounds`]), 0x4 (whether it has an engine, [`ENGINE_START_EVENT`]),
//! [`MODE`] at 0x8, [`CHANCE`] at 0xc, the engine bank at 0x10
//! ([`crate::weapons`]), [`HEARD`] at 0x14 and [`REACH`] at 0x18.
//!
//! # Provenance
//!
//! From the 360 code: the `SoundEffect` system's handler for an entity
//! coming into being (0x82403758, in the system's table at 0x820EB354)
//! fetches the entity's record and, when its word at 0x8 is 1 and a draw
//! of the game's generator in [0, 1) (0x8215E260) is no more than the
//! float at 0xc, posts the sound system (0x83220B28, through 0x82174F88)
//! a message naming the entity, the record's `SoundSet`, the event
//! [`CREATE_EVENT`] (`0x390ADD0A`) and the `Param` wildcard
//! ([`m2_assets::animation_table::ANY`]). Mode 2 is told apart there as
//! well (a flag on the tracked emitter it registers), and the handler for
//! an entity going (0x82403918) stops its sounds.
//!
//! From samples: all 59 blast templates' records have mode 1 and chance
//! 1.0; their sound sets' only `Sounds` rows are for [`CREATE_EVENT`]
//! (41 of the 59 templates), each naming a cue of the world's cue tables
//! (the grenade's `19152b89`, four variants heard 50 to 200 m; the
//! satellite bomb's `2d26c50f`, a mixed cue of seven tracks heard 100 to
//! 1000 m). The event's name is not known.

use std::collections::HashMap;

use m2_assets::Layer;

use crate::tuning::Table;

/// The `Sounds` table event a `SoundEffect` plays as its entity comes
/// into being (name unknown; the hash the 360 code posts, 0x82403758).
pub const CREATE_EVENT: u32 = 0x390a_dd0a;

/// The `Sounds` table event a pickup's `SoundEffect` set plays as it is
/// taken (name unknown). From the 360 code: taking a pickup (0x8228F5E8,
/// 0x8228FE58) posts the script event `Pickup` (0x8602E37D) naming the
/// pickup; the sound system's handler for script events (0x82360FA8, run
/// from its update 0x8235EFA0) answers it by starting this event on the
/// pickup's own set (its `SoundEffect` record, table 0x83654C50; none,
/// nothing), the `Param` wildcard, at the pickup. From samples: 26 sound
/// sets have a row for it (the ammunition boxes' among them).
pub const TAKEN_EVENT: u32 = 0xa639_af36;

/// The `Sounds` table event a vehicle's set plays as its engine starts
/// (name unknown), and the one it plays as the engine stops.
///
/// From the 360 code: a `SoundEffect` record whose word at 0x4 is 1 or
/// more registers an engine for its entity (0x82403758 takes a node from
/// the pool at 0x831343EC; 0x82367358 hangs a controller of the entity's
/// kind on it: wheeled 0x820D9E90, others 0x820D9E78/A8/C0). The sound
/// system's handler for seat changes (0x82360400) starts the engine as
/// the player gets in to drive (0x82367800 with its third argument set):
/// it starts one of two sets of loops on the vehicle's set
/// (`82577648`, `514c1ed1` and `6477ffe6`, or `fbf9a4b4`, `8a55f77d` and
/// `a1ced25e`, by its second and fourth arguments) and then this event.
/// As the player gets out it stops the engine (0x82367E30): those loops
/// stop and [`ENGINE_OFF_EVENT`] starts, the controller's own loops stop
/// after (0x82369AF8 on a wheeled one: `3c4e6551`, `a99c0571`,
/// `376de087`). Every message names the vehicle, its set and the `Param`
/// wildcard.
///
/// From samples: of the 58 sets of templates with that word set, 53
/// have a row for each. 49 starts and 49 stops find sounds; the other
/// four of each (the helicopters' and the trailer's) name `ba71c11c`, a
/// cue no table or bank holds, which the template database also gives
/// as many records' empty value: silent. Most are plain cues whose
/// record words carry a high half ([`m2_assets::voice::PlainCue`]), and
/// all 64 such starts and stops last as long as their cue says
/// (`plaincuelen` example); the rest are mixed cues (the Semi's).
pub const ENGINE_START_EVENT: u32 = 0x72b6_619e;
/// The `Sounds` table event of a vehicle's horn (name unknown).
///
/// From the 360 code: the player's vehicle update (0x8242E7A0) starts it
/// on the vehicle's set as the driver's horn button goes down and stops
/// it as it comes up; getting out stops it too (0x82360400). AI drivers
/// sound another event, `6c15643b` (0x822BE7C0).
///
/// From samples: 54 sound sets have a row for it, and 53 of their cues
/// find a sound (volume 0.85, heard 10 to 100 m on those read).
pub const HORN_EVENT: u32 = 0xfc70_3e23;
pub const ENGINE_OFF_EVENT: u32 = 0xc8f1_7f2a;

/// `SoundEffect`'s field naming its sound set.
const SET: &str = "SoundSet";
/// When it plays (name unknown): 1 as its entity comes into being, 2
/// looped on a tracked emitter (from the 360 code, 0x82403758).
const MODE: u32 = 0x29f1_5442;
/// The chance it plays at all (name unknown; 0x82403758 draws against it).
const CHANCE: u32 = 0x4e97_de03;
/// Whether its entity has an engine (name unknown, word 0x4: 1 or more
/// registers one, 0x82403758; see [`ENGINE_START_EVENT`]).
const ENGINE: u32 = 0xc433_22c3;
/// Whether it registers a reach (name unknown, word 0x14), and the reach
/// itself (word 0x18): passed to the system at 0x83656624 by 0x82403758,
/// not yet followed further.
const HEARD: u32 = 0xd932_985b;
const REACH: u32 = 0x1195_7817;

/// One template's `SoundEffect` record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnSound {
    /// Its sound set (0 for none).
    pub set: u32,
    /// When it plays: 1 as its entity comes into being.
    pub mode: u32,
    /// The chance it plays, 0 to 1.
    pub chance: f32,
    /// The reach it registers, if its flag says to (not yet used).
    pub reach: Option<f32>,
    /// Its entity has an engine ([`ENGINE_START_EVENT`]).
    pub engine: bool,
}

impl SpawnSound {
    /// The sound set it plays as its entity comes into being, for a draw
    /// of `draw` in [0, 1): as 0x82403758 has it, mode 1 and `draw` no
    /// more than its chance. `None` when it plays nothing then (its set
    /// is 0, its mode another, or the draw misses).
    pub fn on_create(&self, draw: f32) -> Option<u32> {
        (self.mode == 1 && draw <= self.chance && self.set != 0).then_some(self.set)
    }
}

/// Every template's `SoundEffect` record, by template id (the first, of
/// a template with several).
pub fn read(db: &Layer<'_>) -> HashMap<u32, SpawnSound> {
    let t = Table::new(db, "SoundEffect");
    let set = m2_assets::name_hash(SET);
    t.templates()
        .filter_map(|id| {
            let r = *t.of_template(id).first()?;
            Some((
                id,
                SpawnSound {
                    set: t.word(r, set).unwrap_or(0),
                    mode: t.word(r, MODE).unwrap_or(0),
                    chance: t.float(r, CHANCE).unwrap_or(0.0),
                    reach: (t.word(r, HEARD) == Some(1))
                        .then(|| t.float(r, REACH))
                        .flatten(),
                    engine: t.word(r, ENGINE).is_some_and(|n| n as i32 >= 1),
                },
            ))
        })
        .collect()
}

/// What a message one of an object's model states sends
/// ([`crate::model::StateMessage`]) has the sound system do with the
/// object's sound set (its `SoundEffect` record's `SoundSet`), in order:
/// start (`true`) or stop (`false`) the `Sounds` table's cue for each
/// event, the message's second operand as the row's `Param`. Empty for a
/// message the sound system doesn't hear.
///
/// From the 360 code: the sound system's handler for the object event
/// queue (0x8235FCE8, run from its update 0x8235EFA0) fetches the
/// object's `SoundEffect` record (table 0x83654C50; none, nothing) and
/// posts the sound system (0x83220B28) one or two messages {0 to start,
/// 1 to stop; the object; 9; the set; the event; the operand}, by the
/// message's hash. The messages' and events' names are not known; of
/// the island's models, a building's collapse sends `de60c8e2` (186
/// commands) and its wreck `73aa04b8`, and the buildings' sound sets
/// answer `9cda7642` (a long cue, its tracks to 3.5 s) and `0519d48b`
/// (PC data, `sendmessages` and `collapsesounds` examples).
pub fn object_event(message: u32) -> &'static [(bool, u32)] {
    match message {
        0xde60_c8e2 | 0x020f_0fba => &[(true, 0x9cda_7642), (true, 0x4fa7_1143)],
        0x73aa_04b8 | 0xfc2a_e8f0 => &[(false, 0x9cda_7642), (true, 0x0519_d48b)],
        0x1bbf_3186 | 0x4fa0_90ae => &[(true, 0x6e3f_ade6), (true, 0x6bcd_f63f)],
        0x8435_194c | 0x967b_24e4 => &[(false, 0x6e3f_ade6), (true, 0xd881_a717)],
        0x21ed_b37f => &[(true, 0xdce3_d8a8), (true, 0xd40b_8901)],
        0x3e5b_a553 => &[(false, 0xdce3_d8a8), (true, 0x2584_cfd1)],
        0x5a96_9765 => &[(true, 0x4717_a5db)],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collapse_starts_its_rumble_and_its_wreck_stops_it() {
        assert_eq!(
            object_event(0xde60_c8e2),
            &[(true, 0x9cda_7642), (true, 0x4fa7_1143)]
        );
        assert_eq!(
            object_event(0x73aa_04b8),
            &[(false, 0x9cda_7642), (true, 0x0519_d48b)]
        );
        assert!(object_event(0x1234_5678).is_empty());
    }

    fn grenade() -> SpawnSound {
        SpawnSound {
            set: 0x701d_113d,
            mode: 1,
            chance: 1.0,
            reach: Some(125.0),
            engine: false,
        }
    }

    #[test]
    fn mode_one_plays_its_set_whenever_the_draw_is_within_its_chance() {
        let g = grenade();
        assert_eq!(g.on_create(0.0), Some(0x701d_113d));
        assert_eq!(g.on_create(0.999), Some(0x701d_113d));
        let half = SpawnSound { chance: 0.5, ..g };
        assert_eq!(half.on_create(0.5), Some(0x701d_113d));
        assert_eq!(half.on_create(0.51), None);
    }

    #[test]
    fn other_modes_and_no_set_play_nothing_as_it_comes_into_being() {
        let g = grenade();
        assert_eq!(SpawnSound { mode: 0, ..g }.on_create(0.0), None);
        assert_eq!(SpawnSound { mode: 2, ..g }.on_create(0.0), None);
        assert_eq!(SpawnSound { set: 0, ..g }.on_create(0.0), None);
    }

    #[test]
    fn the_create_event_is_the_hash_the_handler_posts() {
        assert_eq!(CREATE_EVENT, 0x390a_dd0a);
        assert_ne!(CREATE_EVENT, m2_assets::animation_table::ANY);
    }
}
