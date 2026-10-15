//! Which cue a gun plays when it fires: the front end's `Sounds` table.
//!
//! A gun's template names a `SoundSet` (its `SoundEffect`'s field: the
//! carbine's and the combat rifle's `01b47c45`, the assault rifles'
//! `7473fe68`). The `Sounds` table (an animation table, `9cd06f1d`, in
//! the resident blocks of `vz.wad` and `shell.wad` alike) has the columns
//! `SoundSet`, `EventHandle`, `Param` and `SoundID`: a row gives the cue
//! (`SoundID`, a cue of the world's cue tables, plain or a mix of several
//! records, [`m2_assets::MixedCue`]) a sound set plays for an event. The
//! events named so far are [`FIRE`] (`weapon_fire`: the gun's own bank,
//! `wpn_combatrifle`'s three loud shots for the carbine), [`FIRE_NPC`]
//! (`weapon_fire_npc`: a cue of the resident `wpn_shared` bank),
//! [`DRY_FIRE`] (`weapon_dryfire`), `reload` and `stow`; two more
//! (`450459d9`, `a639af36`) are not named. `Param` is [`ANY`] on most rows;
//! the assault rifles' NPC shot has six rows with other `Param`s (not
//! named), which pick between three cues.
//!
//! # Provenance
//!
//! From samples: the table's columns are its own names; the events'
//! names are found by hashing guesses (`weapon_fire` hashes to the
//! carbine's row whose cue is its bank's loudest three-variant record);
//! that a gun plays `weapon_fire` when the hero fires and
//! `weapon_fire_npc` when anyone else does is inferred from the names. Not
//! checked against the game's code.

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::animation_table::{ANIMATION_TABLE_TYPE, ANY, AnimationTable};
use m2_assets::name_hash;

use crate::assets::{AssetSource, Install};

/// The `Sounds` table's name hash.
pub const SOUNDS_TABLE: u32 = 0x9cd0_6f1d;
/// The events a gun's sound set answers.
pub const FIRE: &str = "weapon_fire";
pub const FIRE_NPC: &str = "weapon_fire_npc";
pub const DRY_FIRE: &str = "weapon_dryfire";

/// Each sound set's cue per event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GunCues {
    cues: HashMap<(u32, u32), u32>,
}

impl GunCues {
    /// From the table's rows (`SoundSet`, `EventHandle`, `Param`,
    /// `SoundID`): of several rows for a set and event, the one whose
    /// `Param` is [`ANY`], else the first. Rows naming no set (the
    /// [`ANY`] set's, which key the set in `Param` instead) are left out.
    pub fn from_rows(rows: impl IntoIterator<Item = [u32; 4]>) -> Self {
        let mut cues = HashMap::new();
        let mut plain = std::collections::HashSet::new();
        for [set, event, param, cue] in rows {
            if set == ANY || cue == ANY || cue == 0 {
                continue;
            }
            if param == ANY {
                if plain.insert((set, event)) {
                    cues.insert((set, event), cue);
                }
            } else if !plain.contains(&(set, event)) {
                cues.entry((set, event)).or_insert(cue);
            }
        }
        Self { cues }
    }

    /// From the parsed table; `None` if it lacks one of its columns.
    pub fn from_table(table: &AnimationTable) -> Option<Self> {
        let cols = [
            table.column("SoundSet")?,
            table.column("EventHandle")?,
            table.column("Param")?,
            table.column("SoundID")?,
        ];
        Some(Self::from_rows((0..table.rows()).filter_map(|r| {
            Some([
                table.cell(r, cols[0])?,
                table.cell(r, cols[1])?,
                table.cell(r, cols[2])?,
                table.cell(r, cols[3])?,
            ])
        })))
    }

    /// Reads the table from the install's world archive.
    pub fn read(install: &Install) -> Result<Self, String> {
        let mut source = install.open_world().map_err(|e| e.to_string())?;
        Self::from_source(&mut source)
    }

    /// Reads the table from an archive holding it.
    pub fn from_source<R: Read + Seek>(source: &mut AssetSource<R>) -> Result<Self, String> {
        let data = source
            .asset(SOUNDS_TABLE, ANIMATION_TABLE_TYPE)
            .map_err(|e| e.to_string())?;
        let table = AnimationTable::parse(&data).map_err(|e| e.to_string())?;
        Self::from_table(&table).ok_or_else(|| "the Sounds table lacks a column".to_owned())
    }

    /// The cue sound set `set` plays for `event` ([`FIRE`], ...).
    pub fn cue(&self, set: u32, event: &str) -> Option<u32> {
        self.cues.get(&(set, name_hash(event))).copied()
    }

    /// The cue a gun of sound set `set` fires with: the hero's
    /// ([`FIRE`]) or anyone else's ([`FIRE_NPC`], else [`FIRE`]).
    pub fn shot(&self, set: u32, npc: bool) -> Option<u32> {
        if npc {
            self.cue(set, FIRE_NPC).or_else(|| self.cue(set, FIRE))
        } else {
            self.cue(set, FIRE)
        }
    }

    /// Sound sets with a cue for `event`.
    pub fn sets_with(&self, event: &str) -> usize {
        let e = name_hash(event);
        self.cues.keys().filter(|(_, ev)| *ev == e).count()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The event names hash to the table's `EventHandle`s (read from a
    // legally installed copy's table: the carbine's rows).
    #[test]
    fn the_event_names_hash_to_the_tables_handles() {
        assert_eq!(name_hash(FIRE), 0x8f87_f89c);
        assert_eq!(name_hash(FIRE_NPC), 0x3842_7a98);
        assert_eq!(name_hash(DRY_FIRE), 0x6eea_9c0f);
        assert_eq!(name_hash("reload"), 0x8190_ec52);
        assert_eq!(name_hash("stow"), 0xd8db_0afe);
    }

    // The carbine's rows as the table has them, and the assault rifle's
    // NPC shot, keyed by `Param` only.
    #[test]
    fn a_set_fires_its_own_cue_and_its_npc_cue() {
        let (carbine, ak) = (0x01b4_7c45, 0x7473_fe68);
        let cues = GunCues::from_rows([
            [carbine, 0x3842_7a98, ANY, 0x1fc5_1b48],
            [carbine, 0x8f87_f89c, ANY, 0x33a0_71cc],
            [carbine, 0x6eea_9c0f, ANY, 0x18be_fe36],
            [ANY, 0x8190_ec52, carbine, 0x176b_614b],
            [ak, 0x3842_7a98, 0x6333_f2bc, 0xd8ce_1427],
            [ak, 0x3842_7a98, 0x3d31_7853, 0x8827_8c4f],
            [ak, 0x8f87_f89c, ANY, 0x4988_08cf],
        ]);
        assert_eq!(cues.shot(carbine, false), Some(0x33a0_71cc));
        assert_eq!(cues.shot(carbine, true), Some(0x1fc5_1b48));
        assert_eq!(cues.cue(carbine, DRY_FIRE), Some(0x18be_fe36));
        // The reload row keys the set in `Param`: not read as a set's.
        assert_eq!(cues.cue(carbine, "reload"), None);
        // Of the `Param`-keyed rows, the first.
        assert_eq!(cues.shot(ak, true), Some(0xd8ce_1427));
        assert_eq!(cues.shot(ak, false), Some(0x4988_08cf));
        // An unknown set fires nothing.
        assert_eq!(cues.shot(0x1234_5678, false), None);
        assert_eq!(cues.sets_with(FIRE), 2);
    }

    // A plain row overrides the `Param`-keyed rows before it.
    #[test]
    fn a_plain_row_wins_over_keyed_ones() {
        let cues = GunCues::from_rows([[1, 2, 7, 10], [1, 2, ANY, 20], [1, 2, 8, 30]]);
        assert_eq!(cues.cues.get(&(1, 2)), Some(&20));
    }
}
