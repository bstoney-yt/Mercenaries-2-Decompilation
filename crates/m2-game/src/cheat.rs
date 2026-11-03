//! The developers' cheat menu the game's own scripts carry
//! (`MrxCheatBootstrap`), and the loadouts its mission skip hands out
//! (`WifCheatStockpile`).
//!
//! From samples (the game's scripts, run by [`crate::mission::Mission`]):
//! the menu is pure script, `MrxCheatBootstrap.DisplayOptions`, and the
//! `Cheat` table's one member points at it. Its root page on PC reads
//! "Welcome to the Cheat Menu." with Add cash, Add fuel, Add support,
//! Modify attitude, Unlock all landing zones, Dispense all rewards and
//! Close this menu; the module also holds pages for the mission skip, the
//! task tree, the factions and completing the current contract. Each page
//! is one of the scripts' dialog boxes, which the HUD already puts up.
//! The root page's text and the module's member names were described by a
//! third-party modding project (loganw234/Mercenaries2, MIT) that opened
//! the menu on the PC game; nothing of theirs is used here, and the menu
//! itself runs from the user's own data.
//!
//! `WifCheatStockpile` is keyed by contract or job (`AllCon001`...) and
//! gives each one `nCash`, `nFuel`, `tEquipment` and `tSupport` (item key
//! to count): what she is meant to hold on arriving there by the skip.
//!
//! Unknown: how the developers opened the menu (no retail path calls it
//! that the scripts show; the 360 program is still to be traced for a pad
//! combination or INI switch). The key m2-alpha opens it with is invented.

use std::collections::BTreeMap;

use mlua::{Lua, Table, Value};

/// Lua that puts the scripts' cheat menu up: `MrxCheatBootstrap.
/// DisplayOptions()`, else the `Cheat` table's. Errors if the scripts
/// have neither.
pub const OPEN_MENU: &str = r#"
local f
local c = rawget(_G, "MrxCheatBootstrap")
if type(c) == "table" then f = rawget(c, "DisplayOptions") end
if type(f) ~= "function" then
    local t = rawget(_G, "Cheat")
    if type(t) == "table" then f = rawget(t, "DisplayOptions") end
end
if type(f) ~= "function" then
    error("the scripts have no MrxCheatBootstrap.DisplayOptions", 0)
end
f()
"#;

/// One contract's entry of `WifCheatStockpile`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stockpile {
    pub cash: f64,
    pub fuel: f64,
    /// Support item key to count (`tSupport`).
    pub support: BTreeMap<String, i64>,
    /// How many entries `tEquipment` has (their layout is not read yet).
    pub equipment: usize,
}

impl Stockpile {
    /// One line for a log: cash, fuel and the support total.
    pub fn summary(&self) -> String {
        let items: i64 = self.support.values().sum();
        format!(
            "cash {:.0}, fuel {:.0}, {} support items of {} kinds, {} equipment entries",
            self.cash,
            self.fuel,
            items,
            self.support.len(),
            self.equipment
        )
    }
}

/// The scripts' stockpile table, if they have loaded it.
fn table(lua: &Lua) -> Option<Table> {
    let globals = lua.globals();
    if let Ok(t) = globals.raw_get::<Table>("WifCheatStockpile") {
        return Some(t);
    }
    // Not imported as a global yet: the scripts' own loader.
    let import: mlua::Function = globals.get("import").ok()?;
    match import.call::<Value>("WifCheatStockpile").ok()? {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

/// `WifCheatStockpile[contract]`, if the scripts have one for it.
pub fn stockpile(lua: &Lua, contract: &str) -> Option<Stockpile> {
    let entry: Table = table(lua)?.raw_get(contract).ok()?;
    let number = |k: &str| entry.raw_get::<f64>(k).unwrap_or(0.0);
    let mut support = BTreeMap::new();
    if let Ok(t) = entry.raw_get::<Table>("tSupport") {
        for (k, v) in t.pairs::<String, f64>().flatten() {
            support.insert(k, v as i64);
        }
    }
    let equipment = entry
        .raw_get::<Table>("tEquipment")
        .map(|t| t.pairs::<Value, Value>().count())
        .unwrap_or(0);
    Some(Stockpile {
        cash: number("nCash"),
        fuel: number("nFuel"),
        support,
        equipment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua_with(code: &str) -> Lua {
        let lua = Lua::new();
        lua.load(code).exec().unwrap();
        lua
    }

    #[test]
    fn a_contracts_stockpile_is_read_from_the_scripts_table() {
        let lua = lua_with(
            r#"WifCheatStockpile = { AllCon001 = { nCash = 250000, nFuel = 300,
                tEquipment = { "a", "b" },
                tSupport = { artillery = 2, gl = 1 } } }"#,
        );
        let s = stockpile(&lua, "AllCon001").unwrap();
        assert_eq!(s.cash, 250_000.0);
        assert_eq!(s.fuel, 300.0);
        assert_eq!(s.equipment, 2);
        assert_eq!(s.support.get("artillery"), Some(&2));
        assert_eq!(s.support.get("gl"), Some(&1));
        assert_eq!(
            s.summary(),
            "cash 250000, fuel 300, 3 support items of 2 kinds, 2 equipment entries"
        );
        assert_eq!(stockpile(&lua, "PirCon999"), None);
    }

    #[test]
    fn no_stockpile_without_the_scripts_table() {
        let lua = Lua::new();
        assert_eq!(stockpile(&lua, "AllCon001"), None);
    }

    #[test]
    fn the_menu_opens_through_the_bootstrap_else_the_cheat_table() {
        let lua = lua_with(
            "opened = nil
             MrxCheatBootstrap = { DisplayOptions = function() opened = 'bootstrap' end }",
        );
        lua.load(OPEN_MENU).exec().unwrap();
        assert_eq!(lua.globals().get::<String>("opened").unwrap(), "bootstrap");

        let lua = lua_with("Cheat = { DisplayOptions = function() opened = 'cheat' end }");
        lua.load(OPEN_MENU).exec().unwrap();
        assert_eq!(lua.globals().get::<String>("opened").unwrap(), "cheat");

        let err = Lua::new().load(OPEN_MENU).exec().unwrap_err();
        assert!(err.to_string().contains("no MrxCheatBootstrap"));
    }
}
