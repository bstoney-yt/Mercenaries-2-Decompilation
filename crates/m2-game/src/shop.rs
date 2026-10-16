//! The shop as the scripts run it (`MrxShop`): a faction's contact opens
//! it from their menu (`MrxShop.Open(vendor)`, the briefing menu's
//! "[Briefing.Shop]"), which fills the engine's shop interface
//! (`Hud.Shop`) with every support item and piece of equipment the
//! vendor's faction sells, priced by the faction's mood
//! (`MrxFactionManager.GetPriceScale`), locked or not. A purchase is the
//! scripts' own `_ShopSelection(id, quantity)`, the callback the shop was
//! given: it checks her cash, adds the item to her stockpile
//! (`MrxPmc.AddSupportQty`) and takes the price (`MrxPmc.AddCashQty`).
//! Nothing of the catalogue is copied here.
//!
//! From the scripts: the calls `MrxShop.Open` makes and the fields of
//! each item it adds (`AddItemFull`). How the engine calls the purchase
//! callback, `(id, quantity)`, is inferred from `_ShopSelection`'s two
//! parameters: it compares the first with `"Locked"` and multiplies the
//! price by the second. Not checked against the 360 code.

use mlua::{Lua, Table, Value};

/// The engine's shop interface, as a table of methods (`Hud.Shop`, kept as
/// `__m2_hud_shop` too): it keeps what the scripts put in it in
/// `__m2_shop`.
pub const HUD_SHOP: &str = r#"
local S = {}
local function fresh() __m2_shop = { open = false, items = {} } end
fresh()
function S.Create(self, t) fresh() end
function S.SetCallback(self, t)
    if type(t) == "table" then __m2_shop.fCallback = t.fCallback end
end
function S.SetCloseCallback(self, t)
    if type(t) == "table" then __m2_shop.fClose = t.fCallback end
end
function S.AddItemFull(self, t)
    if type(t) == "table" then table.insert(__m2_shop.items, t) end
end
function S.Commence(self, t) __m2_shop.open = true end
function S.Close(self, t) __m2_shop.open = false end
__m2_hud_shop = S
return S
"#;

/// Opens faction `__m2_faction`'s shop as its contact's menu does, with a
/// stand-in for the contact (the scripts ask it only its faction and
/// whether it keeps a vehicle shop of its own). The engine's interface is
/// put back first: the `Hud` the scripts see by then is not always the
/// table it was installed in. Evaluates to whether the shop came up.
pub const OPEN: &str = r#"
rawset(Hud, "Shop", __m2_hud_shop)
local vendor = {}
function vendor:GetFaction() return __m2_faction end
function vendor:HasCustomVehicleShop() return false end
MrxShop.Open(vendor)
return __m2_shop.open == true
"#;

/// One thing on sale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShopItem {
    /// What a purchase names (`sId`): the support item's key in
    /// `tSupportData` (`"clusterbomb"`) or the equipment's, or `"Locked"`.
    pub id: String,
    /// The string keys of its name and description (`"[Shop.LockedItem]"`
    /// when locked), its key (`sRawName`), and its picture.
    pub name: String,
    pub description: String,
    pub raw_name: String,
    pub texture: String,
    /// Its price here (`nCashCost`, the faction's scale applied).
    pub cash: f64,
    /// How many she holds, and the most she may.
    pub stock: i64,
    pub max_stock: i64,
    /// A fuel tank (equipment), and the fuel it holds.
    pub fuel_tank: bool,
    pub fuel: f64,
    pub unlocked: bool,
    /// Shown as new.
    pub new: bool,
}

/// The shop as the scripts left it, if they have put one up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shop {
    /// Up (`Commence`d and not closed).
    pub open: bool,
    pub items: Vec<ShopItem>,
}

impl Shop {
    /// The shop the scripts put up in `lua` (empty when there is none).
    pub fn read(lua: &Lua) -> mlua::Result<Shop> {
        let Value::Table(shop) = lua.globals().get::<Value>("__m2_shop")? else {
            return Ok(Shop::default());
        };
        let mut items = Vec::new();
        if let Some(list) = shop.get::<Option<Table>>("items")? {
            for t in list.sequence_values::<Table>() {
                items.push(item(&t?));
            }
        }
        Ok(Shop {
            open: shop.get::<Option<bool>>("open")?.unwrap_or(false),
            items,
        })
    }
}

fn item(t: &Table) -> ShopItem {
    let s = |k: &str| {
        t.get::<Option<String>>(k)
            .ok()
            .flatten()
            .unwrap_or_default()
    };
    let n = |k: &str| t.get::<Option<f64>>(k).ok().flatten().unwrap_or(0.0);
    let b = |k: &str| t.get::<Option<bool>>(k).ok().flatten().unwrap_or(false);
    ShopItem {
        id: s("sId"),
        name: s("sName"),
        description: s("sDescription"),
        raw_name: s("sRawName"),
        texture: s("sTexture"),
        cash: n("nCashCost"),
        stock: n("nCurrentStock") as i64,
        max_stock: n("nMaxStock") as i64,
        fuel_tank: b("bFuelTank"),
        fuel: n("nFuelQuantity"),
        unlocked: b("bUnlocked"),
        new: b("bMarkAsNew"),
    }
}

/// Installs the engine's shop interface as `Hud.Shop` in `hud`.
pub fn install(lua: &Lua, hud: &Table) -> mlua::Result<()> {
    let shop: Table = lua.load(HUD_SHOP).set_name("=m2-shop").eval()?;
    hud.set("Shop", shop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shop like `MrxShop.Open` fills, with `_ShopSelection`'s checks:
    /// the price against her cash, then stock up and cash down.
    const SCRIPTS: &str = r#"
        cash, stock = 1000, {}
        MrxShop = {}
        function MrxShop.Open(vendor)
            local me = 1
            Hud.Shop:Create({ uPlayer = me })
            Hud.Shop:SetCallback({ uPlayer = me, fCallback = MrxShop.Select, tCallbackData = {} })
            Hud.Shop:SetCloseCallback({ uPlayer = me, fCallback = function() closed = true end })
            local scale = vendor:GetFaction() == "Pmc" and 1 or 2
            Hud.Shop:AddItemFull({ uPlayer = me, sId = "crate", sName = "[Support.Crate]",
                sRawName = "crate", nCashCost = 400 * scale, nCurrentStock = 0,
                nMaxStock = 9, bFuelTank = false, bUnlocked = true, bMarkAsNew = true })
            Hud.Shop:AddItemFull({ uPlayer = me, sId = "Locked", sName = "[Shop.LockedItem]",
                sRawName = "nuke", nCashCost = 0, bUnlocked = false })
            Hud.Shop:Commence({ uPlayer = me })
        end
        function MrxShop.Select(id, n)
            if id == "Locked" then return false end
            local price = 400 * n
            if price > cash then return false end
            stock[id] = (stock[id] or 0) + n
            cash = cash - price
            return true
        end
    "#;

    fn lua() -> Lua {
        let lua = Lua::new();
        let hud = lua.create_table().unwrap();
        install(&lua, &hud).unwrap();
        lua.globals().set("Hud", hud).unwrap();
        lua.load(SCRIPTS).exec().unwrap();
        lua
    }

    #[test]
    fn the_scripts_shop_is_read_as_they_fill_it() {
        let lua = lua();
        assert!(!Shop::read(&lua).unwrap().open);
        lua.globals().set("__m2_faction", "Pmc").unwrap();
        assert!(lua.load(OPEN).eval::<bool>().unwrap());
        let shop = Shop::read(&lua).unwrap();
        assert!(shop.open);
        assert_eq!(shop.items.len(), 2);
        let crate_ = &shop.items[0];
        assert_eq!(
            (
                crate_.id.as_str(),
                crate_.cash,
                crate_.max_stock,
                crate_.unlocked,
                crate_.new
            ),
            ("crate", 400.0, 9, true, true)
        );
        assert_eq!(
            (shop.items[1].id.as_str(), shop.items[1].raw_name.as_str()),
            ("Locked", "nuke")
        );
        // Another faction's prices.
        lua.globals().set("__m2_faction", "Pirate").unwrap();
        lua.load(OPEN).exec().unwrap();
        assert_eq!(Shop::read(&lua).unwrap().items[0].cash, 800.0);
    }
}
