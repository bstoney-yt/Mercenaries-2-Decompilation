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
    /// A free item the support menu lists with no count of its own
    /// (`bActive`: `AddFreebie` puts it on the menu even when it finds no
    /// quantity, as for the verify objective's `Extraction_OC`, whose
    /// `nFreebieQty` is nil).
    pub active: bool,
}

/// Every item of `tSupportData` (the shops'), then of `tFreebieData`
/// (given free only: a verify objective's `Extraction_OC` helicopter), by
/// key.
pub fn items(lua: &Lua) -> mlua::Result<Vec<SupportItem>> {
    let globals = lua.globals();
    let Value::Table(data) = globals.get::<Value>("MrxSupportData")? else {
        return Ok(Vec::new());
    };
    let mut tables = Vec::new();
    for name in ["tSupportData", "tFreebieData"] {
        if let Value::Table(t) = data.get::<Value>(name)? {
            tables.push(t);
        }
    }
    let pmc = globals.get::<Value>("MrxPmc")?;
    // `mlua::Integer` is i32 on 32-bit builds, i64 on 64-bit ones.
    #[allow(clippy::useless_conversion)]
    let count = |f: &str, key: &str| -> i64 {
        let Value::Table(pmc) = &pmc else { return 0 };
        pmc.get::<mlua::Function>(f)
            .and_then(|f| f.call::<Value>(key))
            .ok()
            .and_then(|v| match v {
                Value::Integer(i) => Some(i64::from(i)),
                Value::Number(n) => Some(n as i64),
                _ => None,
            })
            .unwrap_or(0)
    };
    let mut out: Vec<SupportItem> = Vec::new();
    for pair in tables.iter().flat_map(|t| t.pairs::<String, Table>()) {
        let Ok((key, item)) = pair else { continue };
        if out.iter().any(|i| i.key == key) {
            continue;
        }
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
            active: item.get::<Option<bool>>("bActive").ok().flatten() == Some(true),
            key,
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// Lua that calls item `__m2_key` in for character `__m2_owner`, as the
/// PDA does (its support page, `MrxGuiPda`): a fresh support object with
/// the item's name and costs, commenced. Its owner is the character
/// calling, where the catalogue's object names no game object (the
/// designator takes only an object's guid, a userdata). Evaluates to
/// whether `Commence` took it.
pub const CALL_IN: &str = r#"
local item = MrxSupportData.tSupportData[__m2_key]
    or (MrxSupportData.tFreebieData or {})[__m2_key]
if not item or not item.oSupport then return false end
local s = item.oSupport
if type(s.uOwner) ~= "userdata" then s:SetOwner(__m2_owner) end
local o = s:Create(s.uOwner)
o:SetSupportName(s:GetSupportName())
o:SetFuelCost(s:GetFuelCost())
o:SetCashCost(s:GetCashCost())
return o:Commence() ~= false
"#;

/// Who has joined her PMC, as the scripts hold it: the support
/// catalogue's requirements (`MrxSupportData.tRequirementsObtained`:
/// `Copter` for Ewan the helicopter pilot, `Mechanic` for Eva, `Pilot`
/// for Misha the jet pilot; the story's start sets all three false and
/// `MrxStarterManager.CreateStarter` sets each true as its recruit's
/// `HelPmcBoss`, `MecPmcBoss` or `JetPmcBoss` starter comes to the PMC),
/// those starters, and how many catalogue items the requirements let
/// her call in (`MrxSupportData.IsSupportEquippable`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recruits {
    pub copter: bool,
    pub mechanic: bool,
    pub pilot: bool,
    /// The PMC's recruit starters the starter manager holds, of
    /// `HelPmcBoss`, `MecPmcBoss` and `JetPmcBoss`.
    pub starters: Vec<String>,
    /// Catalogue items (`tSupportData`) she may call in, and of them how
    /// many a missing recruit refuses.
    pub equippable: usize,
    pub refused: usize,
}

/// [`Recruits`] now; all false and none where the modules aren't loaded.
pub fn recruits(lua: &Lua) -> mlua::Result<Recruits> {
    let code = r#"
local d = MrxSupportData
local r = d and d.tRequirementsObtained or {}
local m = MrxStarterManager
local starters = {}
for _, name in ipairs({ "HelPmcBoss", "MecPmcBoss", "JetPmcBoss" }) do
    if m and m.GetStarter and m.GetStarter(name) then starters[#starters + 1] = name end
end
local ok, refused = 0, 0
if d and d.tSupportData and d.IsSupportEquippable then
    for key, item in pairs(d.tSupportData) do
        local yes = d.IsSupportEquippable(key)
        if yes then ok = ok + 1
        elseif item.oSupport and item.oSupport.GetRecruit and item.oSupport:GetRecruit() then
            refused = refused + 1
        end
    end
end
return r.Copter == true, r.Mechanic == true, r.Pilot == true, starters, ok, refused
"#;
    let (copter, mechanic, pilot, starters, equippable, refused): (
        bool,
        bool,
        bool,
        Vec<String>,
        usize,
        usize,
    ) = lua.load(code).set_name("recruits").eval()?;
    Ok(Recruits {
        copter,
        mechanic,
        pilot,
        starters,
        equippable,
        refused,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recruits_read_the_requirements_the_starters_and_what_they_refuse() {
        let lua = Lua::new();
        lua.load(
            r#"
            local recruit = { GetRecruit = function(self) return self.r end }
            recruit.__index = recruit
            local function item(r) return { oSupport = setmetatable({ r = r }, recruit) } end
            MrxSupportData = {
                tRequirementsObtained = { Fiona = true, Copter = true, Mechanic = true, Pilot = false },
                tSupportData = { crate = item(nil), mi35 = item("Copter"), mig = item("Pilot") },
            }
            function MrxSupportData.IsSupportEquippable(key)
                local r = MrxSupportData.tSupportData[key].oSupport:GetRecruit()
                return not r or MrxSupportData.tRequirementsObtained[r]
            end
            MrxStarterManager = { GetStarter = function(n) return n ~= "JetPmcBoss" or nil end }
            "#,
        )
        .exec()
        .unwrap();
        let r = recruits(&lua).unwrap();
        assert_eq!(
            r,
            Recruits {
                copter: true,
                mechanic: true,
                pilot: false,
                starters: vec!["HelPmcBoss".into(), "MecPmcBoss".into()],
                equippable: 2,
                refused: 1,
            }
        );
    }

    #[test]
    fn recruits_are_none_before_the_modules_load() {
        assert_eq!(recruits(&Lua::new()).unwrap(), Recruits::default());
    }
}
