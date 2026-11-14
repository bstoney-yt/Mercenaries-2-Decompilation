//! The engine sounds' gearbox: per sound set, the gears an engine cue's
//! speed and revs are measured against, from the table `11bbee64`.
//!
//! An engine cue's curves read its vehicle's speed and revs as fractions
//! of a top speed and top revs ([`crate::audio::PARAM_SPEED`],
//! [`crate::audio::PARAM_REVS`]). Those tops are not the vehicle's
//! physics: they come from a table of the same format as the animation
//! tables ([`m2_assets::animation_table`]), keyed by the vehicle's
//! `SoundEffect` sound set ([`crate::spawn_sound::SpawnSound::set`]) and
//! the gear, whose columns name what they hold (`SoundSet`, `Dummy`,
//! `Gear`, `DownExitRPM`, `DownEntryRPM`, `UpEntryRPM`, `UpExitRPM`,
//! `MinVelocity`, `MaxVelocity`, `MinGearDelay`, `MaxGearDelay`,
//! `NumGears`, `RPMOnRate`, `RPMOffRate`, `RPMBrakeRate`, `RPMShiftRate`,
//! `RPMInertia`, `RPMInertiaTime`, `RPMInertiaPower`,
//! `RPMBlendHalfPoint`, `RPMSideBlendHalfPoint`, `LoadTransitionTime`,
//! `MinCompress`).
//!
//! # Provenance
//!
//! From the 360 code: the sound system's per-entity update (0x8236DF48,
//! from 0x8235CA20 over the `RuntimeSoundEffect` table 0x83656624) runs
//! each engine's controller (0x82367258) with a gearbox record fetched by
//! the entity's sound set (0x82368A80: a cache of 12 records of 0x184
//! bytes, filled by 0x82368C68). The record is read from the database
//! asset `11bbee64` of type `207359c7` (loaded by 0x8235C448 into
//! 0x831343E8), a row per gear: the query is the set and the gear, the
//! rest of the key the table's default, and the best-scoring row wins
//! (0x82411638: a cell equal to the query's scores 2, one equal to the
//! default 1). The gear-0 row gives `NumGears` (at most 6) and the
//! record's tail (`RPMBlendHalfPoint` on); each gear's row gives its
//! columns 3 to 18; the record's top revs are the most of the gears'
//! `UpExitRPM`. The wheeled controller's update (0x823696E8) and the
//! tracked one's (0x82369518) set the cue's speed to |speed| over the top
//! gear's `MaxVelocity` and the revs to the gearbox's rpm over the top
//! revs.
//!
//! From samples: 88 rows; every vehicle template whose `SoundEffect`
//! registers an engine finds rows for its set. The Junker Belmont's set
//! (`b4951f3a`) has one gear to 28 m/s (its physics `MaxSpeed` is 31.9),
//! the Austin semis' (`44e231ab`) one to 28 (physics 25 and 27.8), the
//! Capuchin trucks' (`b951f1df`) one to 23 (physics 22.2); all three top
//! out at 10000 rpm. The three-gear sets are motorbikes' and boats'.
//!
//! Not implemented: the controller's own gear and rpm simulation
//! (0x82368080, 0x823682D0), which the revs numerator comes from.

use std::collections::HashMap;

use m2_assets::animation_table::AnimationTable;

/// The table's name hash (asset type
/// [`m2_assets::animation_table::ANIMATION_TABLE_TYPE`]).
pub const SOUND_GEARS: u32 = 0x11bb_ee64;
/// The most gears a record holds (0x82368C68 caps `NumGears` at 6).
pub const MAX_GEARS: usize = 6;

/// One gear's row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundGear {
    pub down_exit_rpm: f32,
    pub down_entry_rpm: f32,
    pub up_entry_rpm: f32,
    pub up_exit_rpm: f32,
    pub min_velocity: f32,
    pub max_velocity: f32,
}

/// A sound set's gears, as the 360 builds its record (0x82368C68).
#[derive(Clone, Debug, PartialEq)]
pub struct SoundGearbox {
    pub gears: Vec<SoundGear>,
    /// The most of the gears' `UpExitRPM`, and 0 with none above it (the
    /// record's word at 0x8, the revs' divisor).
    pub max_rpm: f32,
}

impl SoundGearbox {
    /// The top gear's `MaxVelocity`: what the engine cue's speed is a
    /// fraction of (0x823696E8: the record's 0x84 array at the gear
    /// count).
    pub fn top_speed(&self) -> Option<f32> {
        self.gears.last().map(|g| g.max_velocity)
    }

    /// The cue's speed parameter for a vehicle moving at `speed` m/s:
    /// |speed| over [`top_speed`](Self::top_speed). The 360 does not
    /// clamp it; the curves end at 1. `None` without a positive top.
    pub fn speed_param(&self, speed: f32) -> Option<f32> {
        let top = self.top_speed().filter(|&t| t > 0.0)?;
        Some(speed.abs() / top)
    }

    /// The cue's revs parameter for `rpm`: over [`max_rpm`](Self::max_rpm).
    pub fn revs_param(&self, rpm: f32) -> Option<f32> {
        (self.max_rpm > 0.0).then(|| rpm / self.max_rpm)
    }

    /// Sound set `set`'s gears in `table`, as 0x82368C68 reads them;
    /// `None` when its gear-0 row is not found or a column is missing.
    pub fn read(table: &AnimationTable, set: u32) -> Option<Self> {
        let col = |name: &str| table.column(name);
        let (c_set, c_gear, c_count) = (col("SoundSet")?, col("Gear")?, col("NumGears")?);
        let f = |row: usize, name: &str| -> Option<f32> {
            Some(f32::from_bits(table.cell(row, col(name)?)?))
        };
        let row_of = |gear: u32| table.best(&[(c_set, Some(set)), (c_gear, Some(gear))]);
        let first = row_of(0)?;
        let count = (table.cell(first, c_count)? as i32).clamp(0, MAX_GEARS as i32) as u32;
        let mut gears = Vec::new();
        for g in 0..count {
            let row = if g == 0 { first } else { row_of(g)? };
            gears.push(SoundGear {
                down_exit_rpm: f(row, "DownExitRPM")?,
                down_entry_rpm: f(row, "DownEntryRPM")?,
                up_entry_rpm: f(row, "UpEntryRPM")?,
                up_exit_rpm: f(row, "UpExitRPM")?,
                min_velocity: f(row, "MinVelocity")?,
                max_velocity: f(row, "MaxVelocity")?,
            });
        }
        let max_rpm = gears.iter().map(|g| g.up_exit_rpm).fold(0.0, f32::max);
        Some(Self { gears, max_rpm })
    }

    /// The gears of each of `sets` that the table has.
    pub fn read_all(
        table: &AnimationTable,
        sets: impl IntoIterator<Item = u32>,
    ) -> HashMap<u32, Self> {
        sets.into_iter()
            .filter_map(|s| Some((s, Self::read(table, s)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::animation_table::ANY;

    /// A synthetic table in the asset's format: the columns the record
    /// reads, a catch-all row 0, a one-gear set and a three-gear set.
    fn table() -> AnimationTable {
        let names: [(&str, u16); 10] = [
            ("SoundSet", 4),
            ("Dummy", 4),
            ("Gear", 2),
            ("DownExitRPM", 3),
            ("DownEntryRPM", 3),
            ("UpEntryRPM", 3),
            ("UpExitRPM", 3),
            ("MinVelocity", 3),
            ("MaxVelocity", 3),
            ("NumGears", 2),
        ];
        let f = |v: f32| v.to_bits();
        let rows: [&[u32]; 6] = [
            &[ANY, ANY, 0, 0, 0, 0, 0, 0, 0, 5],
            &[0xb495_1f3a, ANY, 0, 0, 0, 0, f(10000.0), 0, f(28.0), 1],
            &[0xb951_f1df, ANY, 0, 0, 0, 0, f(10000.0), 0, f(23.0), 1],
            &[
                0x7cd9_4294,
                ANY,
                0,
                f(3000.0),
                f(3000.0),
                f(6000.0),
                f(16000.0),
                0,
                f(10.0),
                3,
            ],
            &[
                0x7cd9_4294,
                ANY,
                1,
                f(3000.0),
                f(3000.0),
                f(5500.0),
                f(14000.0),
                f(10.0),
                f(20.0),
                3,
            ],
            &[
                0x7cd9_4294,
                ANY,
                2,
                f(3000.0),
                f(30000.0),
                f(4000.0),
                f(15000.0),
                f(20.0),
                f(25.0),
                3,
            ],
        ];
        AnimationTable::from_rows(&names, &rows)
    }

    #[test]
    fn one_gear_sets_divide_by_their_max_velocity() {
        let t = table();
        let belmont = SoundGearbox::read(&t, 0xb495_1f3a).unwrap();
        assert_eq!(belmont.gears.len(), 1);
        assert_eq!(belmont.top_speed(), Some(28.0));
        assert_eq!(belmont.max_rpm, 10000.0);
        assert_eq!(belmont.speed_param(-14.0), Some(0.5));
        assert_eq!(belmont.revs_param(2500.0), Some(0.25));
        let capuchin = SoundGearbox::read(&t, 0xb951_f1df).unwrap();
        assert_eq!(capuchin.speed_param(23.0), Some(1.0));
    }

    #[test]
    fn several_gears_use_the_top_gear_and_the_highest_up_exit() {
        let g = SoundGearbox::read(&table(), 0x7cd9_4294).unwrap();
        assert_eq!(g.gears.len(), 3);
        assert_eq!(g.top_speed(), Some(25.0));
        assert_eq!(g.max_rpm, 16000.0);
        assert_eq!(g.gears[1].min_velocity, 10.0);
    }

    #[test]
    fn a_set_without_rows_has_no_gearbox() {
        // Row 0 is never picked (`AnimationTable::best`), so a set the
        // table lacks reads as none rather than as the catch-all's zero
        // top speed.
        assert_eq!(SoundGearbox::read(&table(), 0x1234_5678), None);
    }

    /// The installed table, when `M2_INSTALL` names an install (skipped
    /// otherwise): the three sets of the playtest's engine comparison.
    #[test]
    fn installed_table_gives_the_belmont_semi_and_capuchin_tops() {
        let Ok(root) = std::env::var("M2_INSTALL") else {
            return;
        };
        let install = crate::Install::find(std::path::Path::new(&root)).unwrap();
        let mut source = install.open_world().unwrap();
        let data = source
            .asset(
                SOUND_GEARS,
                m2_assets::animation_table::ANIMATION_TABLE_TYPE,
            )
            .unwrap();
        let t = AnimationTable::parse(&data).unwrap();
        for (set, top) in [
            (0xb495_1f3a, 28.0), // Junker Belmont, Belmont (`RTR`)
            (0x44e2_31ab, 28.0), // Austin and Ogre semis
            (0xb951_f1df, 23.0), // Capuchin trucks (`M35`), Emissary Semi
        ] {
            let g = SoundGearbox::read(&t, set).unwrap();
            assert_eq!(
                (g.gears.len(), g.top_speed(), g.max_rpm),
                (1, Some(top), 10000.0)
            );
        }
    }
}
