//! The support catalogue as the scripts hold it: everything the PDA's
//! shop sells and the support menu calls in (supply crates, vehicles
//! dropped by helicopter, boats, air strikes), with its price in cash and
//! fuel and how many she holds. `MrxSupportData.Init` fills
//! `tSupportData` when the game starts; `MrxPmc` keeps her stockpile,
//! cash and fuel. Nothing of it is copied here: the game reads it from
//! the scripts as they run.
//!
//! Calling an item in is the scripts' own sequence, as the PDA and the
//! support menu run it: a fresh copy of the item's support object
//! (`oSupport:Create(owner)`, given the item's name and costs), then
//! `Commence`, which hands her the item's designator. When the
//! designator is down, the support manager (`MrxSupportManager`) checks
//! her fuel, takes the costs and starts the item's own script: a crate
//! or vehicle flown in, or an aircraft's run.

use mlua::{Lua, Table, Value};

/// One item of the catalogue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SupportItem {
    /// Its key in `tSupportData` (`"clusterbomb"`, `"amx30"`).
    pub key: String,
    /// `sType`: Supply, Light, Heavy, Civilian, Heli, Boat or Airstrike.
    pub kind: String,
    /// The string keys of its name and description, and its icon.
    pub name: String,
    pub description: String,
    pub icon: String,
    /// `nCashCost` and `nFuelCost`.
    pub cash: f64,
    pub fuel: f64,
    /// The script that carries it out (`MrxClusterBomb`,
    /// `MrxCrateDelivery`) and how it is aimed (`"Smoke Designator"`).
    pub module: String,
    pub designator: String,
    /// What brings it (`"Support Vehicle (Mig27)"`, `"Mi26 (PMC) (Driver)"`)
    /// and what it brings, if a delivery (`"Supply Drop (AA)"`).
    pub vehicle: String,
    pub cargo: String,
    /// How many she holds (`MrxPmc.GetSupportQty`), and free ones
    /// (`MrxPmc.GetFreebieQty`).
    pub stock: i64,
    pub free: i64,
}

/// Every item of `tSupportData`, by key.
pub fn items(lua: &Lua) -> mlua::Result<Vec<SupportItem>> {
    let globals = lua.globals();
    let Value::Table(data) = globals.get::<Value>("MrxSupportData")? else {
        return Ok(Vec::new());
    };
    let Value::Table(all) = data.get::<Value>("tSupportData")? else {
        return Ok(Vec::new());
    };
    let pmc = globals.get::<Value>("MrxPmc")?;
    let count = |f: &str, key: &str| -> i64 {
        let Value::Table(pmc) = &pmc else { return 0 };
        pmc.get::<mlua::Function>(f)
            .and_then(|f| f.call::<Value>(key))
            .ok()
            .and_then(|v| match v {
                Value::Integer(i) => Some(i),
                Value::Number(n) => Some(n as i64),
                _ => None,
            })
            .unwrap_or(0)
    };
    let mut out = Vec::new();
    for pair in all.pairs::<String, Table>() {
        let Ok((key, item)) = pair else { continue };
        let support = item.get::<Option<Table>>("oSupport").ok().flatten();
        let s = |t: Option<&Table>, k: &str| {
            t.and_then(|t| t.get::<Option<String>>(k).ok().flatten())
                .unwrap_or_default()
        };
        let designator = support
            .as_ref()
            .and_then(|o| o.get::<Option<Table>>("oDesignator").ok().flatten());
        out.push(SupportItem {
            kind: s(Some(&item), "sType"),
            name: s(Some(&item), "sName"),
            description: s(Some(&item), "sDescription"),
            icon: s(Some(&item), "sIcon"),
            cash: item
                .get::<Option<f64>>("nCashCost")
                .ok()
                .flatten()
                .unwrap_or(0.0),
            fuel: item
                .get::<Option<f64>>("nFuelCost")
                .ok()
                .flatten()
                .unwrap_or(0.0),
            module: s(support.as_ref(), "sModuleName"),
            designator: s(designator.as_ref(), "sDesignationType"),
            vehicle: s(support.as_ref(), "sDeliveryVehicle"),
            cargo: s(support.as_ref(), "sCargoToDeliver"),
            stock: count("GetSupportQty", &key),
            free: count("GetFreebieQty", &key),
            key,
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// Lua that calls item `__m2_key` in for character `__m2_owner`, as the
/// PDA does (its support page, `MrxGuiPda`): a fresh support object with
/// the item's name and costs, commenced. Its owner is the character
/// calling, where the catalogue's object names none. Evaluates to
/// whether `Commence` took it.
pub const CALL_IN: &str = r#"
local item = MrxSupportData.tSupportData[__m2_key]
if not item or not item.oSupport then return false end
local s = item.oSupport
if not s.uOwner then s:SetOwner(__m2_owner) end
local o = s:Create(s.uOwner)
o:SetSupportName(s:GetSupportName())
o:SetFuelCost(s:GetFuelCost())
o:SetCashCost(s:GetCashCost())
return o:Commence() ~= false
"#;
