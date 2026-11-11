//! The natives the retail 360 build leaves empty: debug and tool
//! features compiled out (`Debug.LogError`, `Junk.Dump*`, `Ai.Plan*`,
//! `Sound.SetSourceMusicTransition`, `Graphics.Atmosphere.SetSky`, ...).
//!
//! From the 360 code: the registration tables point all 59 of these at
//! one function, `0x828280B0`, whose whole body is `return 0` (8 bytes:
//! nothing pushed, nothing answered). The list was checked twice: the
//! decompiler's list of natives sharing that entry and the gap table's
//! rows for that address agree name for name.
//!
//! [`install`] gives each a function that answers nothing, unless the port
//! already has its own (`Debug.Printf` and `Debug.Assert` keep logging,
//! a port aid). A path through a value that isn't a table is skipped.

use mlua::{Lua, MultiValue, Result, Table, Value};

/// List of dotted Lua paths for the 59 no-op natives on Xbox 360 retail.
pub const RETAIL_NOOPS: &[&str] = &[
    "Ai.AddRoadException",
    "Ai.Feed",
    "Ai.Plan",
    "Ai.PlanClear",
    "Ai.PlanIterate",
    "Ai.PlanSetConditions",
    "Ai.PlanSetGoal",
    "Ai.RemoveExclusionZone",
    "Ai.RemoveRoadException",
    "Ai.SetExclusionZone",
    "Ai.SetLaneActive",
    "Ai.SetRoadSpawning",
    "Ai.SetSidewalkSpawning",
    "Ai.SetTrafficSpawning",
    "Ai.ShowObjectSpawners",
    "Ai.Talk",
    "Ai.Temp",
    "Ai.Water",
    "Debug.Assert",
    "Debug.GetCallstack",
    "Debug.LogError",
    "Debug.LogInfo",
    "Debug.LogWarning",
    "Debug.Printf",
    "Graphics.Atmosphere.SetSky",
    "Graphics.FuelTrail.Extinguish",
    "Graphics.FuelTrail.Ignite",
    "Gui.AddObjective",
    "Junk.DescribeGuid",
    "Junk.DrawPath",
    "Junk.DumpAssetMemory",
    "Junk.DumpAssets",
    "Junk.DumpAssetsDiff",
    "Junk.DumpMemory",
    "Junk.DumpStats",
    "Junk.DumpTextures",
    "Junk.InstallToHDD",
    "Junk.LoadData",
    "Junk.LoadFunctions",
    "Junk.LoadScript",
    "Junk.Search",
    "Junk.SetQGrey",
    "Junk.UseExistingInstall",
    "Net.SendEvent_AddObjective",
    "Net.SendEvent_RemoveObjective",
    "ObjectState.DebugStateMachine",
    "ObjectState.PrintStateMachine",
    "Pg.GetLoadingStaticLayers",
    "Pg.LoadingStaticLayers",
    "Sound.AddFadeCategory",
    "Sound.AddPitchCategory",
    "Sound.ClearPitchCategories",
    "Sound.ClearSourceMusicTransitions",
    "Sound.SetCinematicMode",
    "Sound.SetSourceEnterMusic",
    "Sound.SetSourceExitMusic",
    "Sound.SetSourceMusicTransition",
    "Sound._SummonEd",
    "Sys.WriteToConsole",
];

/// Install no-op functions for all paths in `RETAIL_NOOPS`.
/// Returns the number of functions actually installed (i.e., where the final key was nil).
pub fn install(lua: &Lua) -> Result<usize> {
    let globals = lua.globals();
    let mut installed = 0;

    'paths: for path in RETAIL_NOOPS {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current: Table = globals.clone();

        // Walk intermediate tables, creating them if missing and nil
        for &part in &parts[..parts.len() - 1] {
            match current.raw_get::<Value>(part)? {
                Value::Nil => {
                    let new_table = lua.create_table()?;
                    current.raw_set(part, &new_table)?;
                    current = new_table;
                }
                Value::Table(t) => {
                    current = t;
                }
                // non-table value: skip this path entirely
                _ => continue 'paths,
            }
        }

        // Every table on the way is there: set the final key to a no-op function
        let last = parts[parts.len() - 1];
        if let Value::Nil = current.raw_get::<Value>(last)? {
            let no_op = lua.create_function(|_, _: MultiValue| Ok(MultiValue::new()))?;
            current.raw_set(last, no_op)?;
            installed += 1;
        }
    }

    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_count_equals_noops() {
        let lua = Lua::new();
        let count = install(&lua).unwrap();
        assert_eq!(count, RETAIL_NOOPS.len());
    }

    #[test]
    fn existing_debug_printf_is_preserved() {
        let lua = Lua::new();
        let globals = lua.globals();

        // Pre-install Debug.Printf with a known value
        let debug = lua.create_table().unwrap();
        debug.raw_set("Printf", "Debug.Printf was here").unwrap();
        globals.raw_set("Debug", debug).unwrap();

        // Install no-ops
        install(&lua).unwrap();

        // Verify Debug.Printf is unchanged
        let debug = globals.raw_get::<Table>("Debug").unwrap();
        let printf_val = debug.raw_get::<Value>("Printf").unwrap();
        assert_eq!(
            printf_val,
            Value::String(lua.create_string("Debug.Printf was here").unwrap())
        );
    }

    #[test]
    fn select_junk_dumpstats_works() {
        let lua = Lua::new();
        install(&lua).unwrap();

        // Run: select('#', Junk.DumpStats(1,2))
        let code = r"
            return select('#', Junk.DumpStats(1,2))
        ";
        let result: Value = lua.load(code).eval().unwrap();
        assert_eq!(result, Value::Integer(0));
    }

    #[test]
    fn nested_graphics_atmosphere_set_sky_works() {
        let lua = Lua::new();
        install(&lua).unwrap();

        // Run: Graphics.Atmosphere.SetSky(1,2,3)
        let code = r"
            Graphics.Atmosphere.SetSky(1,2,3)
        ";
        let result: Value = lua.load(code).eval().unwrap();
        assert_eq!(result, Value::Nil);
    }

    #[test]
    fn a_path_through_a_non_table_is_skipped() {
        let lua = Lua::new();
        lua.globals().raw_set("Junk", 5).unwrap();
        let n = install(&lua).unwrap();
        let junk = RETAIL_NOOPS
            .iter()
            .filter(|p| p.starts_with("Junk."))
            .count();
        assert_eq!(n, RETAIL_NOOPS.len() - junk);
        let v: Value = lua.load("return Junk").eval().unwrap();
        assert_eq!(v, Value::Integer(5));
    }

    #[test]
    fn install_twice_installs_zero_second_time() {
        let lua = Lua::new();

        // First install
        let first = install(&lua).unwrap();
        assert_eq!(first, RETAIL_NOOPS.len());

        // Second install
        let second = install(&lua).unwrap();
        assert_eq!(second, 0);
    }
}
