//! What the scripts change of the spawners attached to an object:
//! `Ai.TweakAttachedSpawners(object, settings)` and
//! `Ai.TweakAttachedSpawnersInGroup(object, group, settings)`.
//!
//! From the 360 code (the natives 8230D450 and 8230D4E0, the settings'
//! reader 8230CE00, the merge 82242558, the apply 822426A0, and the
//! spawners' own routines 82242DC8, 822453D8, 822454D0, 82243110):
//!
//! - The engine keeps one record for each object the scripts have
//!   tweaked. A record has eight slots, one for each spawner group, and
//!   in each slot five settings that are either set or left alone: the
//!   state, the spawn list, the seconds per cycle, the chance not active
//!   and the chance to skip.
//! - The groups are named ([`GROUPS`]); a spawner's own group is its
//!   `SpawnerGroup` in the data (0 to 7, the index here).
//!   `TweakAttachedSpawners` sets every slot, `...InGroup` the slot of
//!   the group named, and nothing for a name that is no group's.
//! - The settings' table is read for five fields only: `SpawnerState`
//!   (one of [`STATES`] by name), `SpawnList` (a list's name),
//!   `SecondsPerCycle`, `ChanceNotActive` and `ChanceSkip` (whole
//!   percents). The scripts also give `SpawnerType`, `RadiusType`,
//!   `ActiveRadius` and `SkipPercentChange`: the 360 never reads those,
//!   and neither does this.
//! - A call is merged over the object's record setting by setting: what
//!   an earlier call set stays unless this one sets it again. A record
//!   with nothing set is dropped.
//! - The spawners attached to an object are those whose owner is that
//!   object: in the data, the spawner records the object's own entity (or
//!   its template) carries. A group set to a state that is off, having
//!   been on, has its spawners stopped and cleared; set on, not having
//!   been, has them started from the object's spawner records of that
//!   group; and every spawner of the object takes its group's settings
//!   (state, the two chances as a fraction of 1, the cycle, the list).
//!   An object not in play keeps its record until it is.
//!
//! Not identified, and kept by hash: two of the groups and three of the
//! states. The scripts use the states `"on"` and `"off"` only.

use m2_assets::name_hash;
use mlua::{Table, Value};

/// The spawner groups, by the hash of their name, in the engine's order
/// (the 360's table at 82B77150): the index is a spawner's `SpawnerGroup`.
pub const GROUPS: [u32; 8] = [
    0x4ff9_f863, // "none"
    0xe6ac_c93c, // "ground"
    0x1b67_2d77, // "balcony"
    0x6727_7c2c, // "rooftop"
    0xb620_4598, // not identified
    0xe643_54f5, // not identified
    0xfb45_79a7, // "aa"
    0x1e97_99d5, // "window"
];

/// The spawner states, by the hash of their name, in the engine's order
/// (the 360's table at 82B77118): `"off"`, `"on"`, and three not
/// identified.
pub const STATES: [u32; 5] = [
    0xad27_7060, // "off"
    0xff27_868e, // "on"
    0x2e22_2925,
    0xf443_4ade,
    0x4454_8b4b,
];

/// `"off"`'s place in [`STATES`].
pub const STATE_OFF: u8 = 0;

/// The group a name is, if it is one's.
pub fn group_of(name: &str) -> Option<usize> {
    let hash = name_hash(name);
    GROUPS.iter().position(|&g| g == hash)
}

/// One group's settings: each set, or left alone (`None`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupTweak {
    /// Its place in [`STATES`].
    pub state: Option<u8>,
    /// A `PopulationList`'s name (`"Spawnlist (VZ Balcony)"`).
    pub spawn_list: Option<String>,
    pub seconds_per_cycle: Option<f32>,
    /// Percent, as the scripts give it: the spawners take a hundredth.
    pub chance_not_active: Option<u8>,
    pub chance_skip: Option<u8>,
}

impl GroupTweak {
    /// Reads a call's settings table: the five fields the engine reads.
    /// A state that is none of [`STATES`], or a field of another kind, is
    /// left alone.
    pub fn read(t: &Table) -> Self {
        let name = |k: &str| t.raw_get::<Option<String>>(k).ok().flatten();
        let number = |k: &str| match t.raw_get::<Value>(k) {
            Ok(Value::Number(n)) => Some(n),
            Ok(Value::Integer(i)) => Some(i as f64),
            _ => None,
        };
        let percent = |k: &str| number(k).map(|n| n.clamp(0.0, 255.0) as u8);
        Self {
            state: name("SpawnerState").and_then(|s| {
                let hash = name_hash(&s);
                STATES.iter().position(|&x| x == hash).map(|i| i as u8)
            }),
            spawn_list: name("SpawnList"),
            seconds_per_cycle: number("SecondsPerCycle").map(|n| n as f32),
            chance_not_active: percent("ChanceNotActive"),
            chance_skip: percent("ChanceSkip"),
        }
    }

    /// Whether it sets nothing.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Takes each setting `over` sets.
    fn merge(&mut self, over: &Self) {
        if over.state.is_some() {
            self.state = over.state;
        }
        if over.spawn_list.is_some() {
            self.spawn_list.clone_from(&over.spawn_list);
        }
        if over.seconds_per_cycle.is_some() {
            self.seconds_per_cycle = over.seconds_per_cycle;
        }
        if over.chance_not_active.is_some() {
            self.chance_not_active = over.chance_not_active;
        }
        if over.chance_skip.is_some() {
            self.chance_skip = over.chance_skip;
        }
    }
}

/// An object's record: its eight groups' settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnerRecord {
    pub groups: [GroupTweak; 8],
}

impl SpawnerRecord {
    /// Merges a call's settings into one group (`Some`), or all eight.
    pub fn merge(&mut self, group: Option<usize>, tweak: &GroupTweak) {
        match group {
            Some(g) => {
                if let Some(slot) = self.groups.get_mut(g) {
                    slot.merge(tweak);
                }
            }
            None => self.groups.iter_mut().for_each(|slot| slot.merge(tweak)),
        }
    }

    /// Whether nothing is set in any group.
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(GroupTweak::is_empty)
    }

    /// Whether a group's spawners are held off: its state set, to
    /// `"off"`.
    pub fn off(&self, group: usize) -> bool {
        self.groups
            .get(group)
            .is_some_and(|g| g.state == Some(STATE_OFF))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tweak(lua: &mlua::Lua, table: &str) -> GroupTweak {
        GroupTweak::read(&lua.load(table).eval::<Table>().unwrap())
    }

    #[test]
    fn the_names_hash_to_the_engines_tables() {
        for (i, name) in [
            (0, "none"),
            (1, "ground"),
            (2, "balcony"),
            (3, "rooftop"),
            (6, "aa"),
            (7, "window"),
        ] {
            assert_eq!(group_of(name), Some(i), "{name}");
        }
        // As the scripts write them: "Ground" is "ground".
        assert_eq!(group_of("Ground"), Some(1));
        assert_eq!(group_of("cellar"), None);
        assert_eq!(name_hash("off"), STATES[0]);
        assert_eq!(name_hash("on"), STATES[1]);
    }

    #[test]
    fn only_the_five_fields_the_engine_reads_are_taken() {
        let lua = mlua::Lua::new();
        // The dangerous building's call: four of its fields do nothing.
        let t = tweak(
            &lua,
            r#"{ SpawnerState = "on", SpawnerType = "Once", RadiusType = "RADIUS_PLAYER_2D",
                ActiveRadius = 100, SkipPercentChange = 100, SpawnList = "Spawnlist (VZ Tower)" }"#,
        );
        assert_eq!(
            t,
            GroupTweak {
                state: Some(1),
                spawn_list: Some("Spawnlist (VZ Tower)".into()),
                ..GroupTweak::default()
            }
        );
        let t = tweak(
            &lua,
            "{ SpawnerState = 'sideways', SecondsPerCycle = 12.5, ChanceNotActive = 30, ChanceSkip = 5 }",
        );
        assert_eq!(
            t,
            GroupTweak {
                seconds_per_cycle: Some(12.5),
                chance_not_active: Some(30),
                chance_skip: Some(5),
                ..GroupTweak::default()
            }
        );
        assert!(tweak(&lua, "{ ActiveRadius = 100 }").is_empty());
    }

    #[test]
    fn calls_merge_setting_by_setting_into_one_group_or_all() {
        let lua = mlua::Lua::new();
        let mut record = SpawnerRecord::default();
        assert!(record.is_empty());
        // As GurJob020 does: everything on from the balcony's list, then
        // the ground group from its own.
        record.merge(
            None,
            &tweak(
                &lua,
                r#"{ SpawnerState = "on", SpawnList = "Spawnlist (VZ Balcony)" }"#,
            ),
        );
        record.merge(
            group_of("Ground"),
            &tweak(&lua, r#"{ SpawnList = "Spawnlist (VZ Ground)" }"#),
        );
        assert_eq!(
            record.groups[1].spawn_list.as_deref(),
            Some("Spawnlist (VZ Ground)")
        );
        // The ground group kept the state the first call set.
        assert_eq!(record.groups[1].state, Some(1));
        assert_eq!(
            record.groups[2].spawn_list.as_deref(),
            Some("Spawnlist (VZ Balcony)")
        );
        // As the dangerous building does once it is cleared.
        record.merge(
            group_of("ground"),
            &tweak(&lua, r#"{ SpawnerState = "off" }"#),
        );
        assert!(record.off(1) && !record.off(2));
        assert!(!record.is_empty());
    }
}
