//! Objects' cash values: `Object.GetCashValue`.
//!
//! From the 360 code: `Object.GetCashValue(object)` (0x8232eaa8) reads the
//! object's record in a 4-byte component table (0x83653d2c, registered
//! by 0x823b4b70) and returns it as a number. An object without one
//! returns nothing, and a call without a guid returns nil.
//!
//! From samples: the table is the template database's `CashValue`
//! component, a single 32-bit field. In vz.wad it is given to the Stingray
//! II (7500) and Phoenix (5000) motorcycles and their driven variants.
//! Other vehicles' templates (the Pony, the garbage truck, the M35) carry
//! 0. An object takes its template's value, and the scripts' one caller
//! (bbea87ce, a toolbox part's handler) pays the value out with
//! `MrxPmc.AddCashQty` when there is one.

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// The template database's `CashValue` records, by template name in lower
/// case.
pub fn template_cash(db: &m2_assets::layer::Layer<'_>) -> HashMap<String, i32> {
    let Some(records) = db.component("CashValue").and_then(|c| c.records()) else {
        return HashMap::new();
    };
    let names = db.names();
    records
        .into_iter()
        .filter_map(|(id, r)| {
            let v = i32::from_le_bytes(r.get(..4)?.try_into().ok()?);
            Some((names.get(&id)?.to_ascii_lowercase(), v))
        })
        .collect()
}

impl ScriptWorld {
    /// An object's cash value: its template's (the template it was placed
    /// as, or for a spawned object the one it is named after).
    pub fn cash_value(&self, guid: u32) -> Option<i32> {
        let template = match self.placed_as.get(&guid) {
            Some(t) => t.clone(),
            None => self.objects.get(&guid)?.name.clone(),
        };
        self.template_cash
            .get(&template.to_ascii_lowercase())
            .copied()
    }
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    add(&lib("Object")?, "GetCashValue", get_cash_value)
}

/// `Object.GetCashValue(object)`.
fn get_cash_value(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(g) = a.first().and_then(crate::events::as_guid) else {
        return Ok(MultiValue::from_vec(vec![Value::Nil]));
    };
    Ok(match w.cash_value(g) {
        Some(v) => MultiValue::from_vec(vec![Value::Number(f64::from(v))]),
        None => MultiValue::new(),
    })
}

#[cfg(test)]
mod tests {
    use crate::api::{Object, Shared, install};
    use crate::script::ScriptHost;
    use mlua::Value;

    #[test]
    fn an_object_answers_its_templates_cash_value() {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.template_cash.insert("stingray ii".into(), 7500);
            w.template_cash.insert("pony (base)".into(), 0);
            // Spawned: named after its template.
            w.insert(1, Object::new("Stingray II", [0.0; 3]));
            // Placed from a layer as a template.
            w.insert(2, Object::new("pmc.pony7", [0.0; 3]));
            w.placed_as.insert(2, "Pony (base)".into());
            w.insert(3, Object::new("crate", [0.0; 3]));
        }
        install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let got = |code: &str| -> Vec<Value> {
            lua.load(format!("return {code}"))
                .eval::<mlua::MultiValue>()
                .unwrap()
                .into_iter()
                .collect()
        };
        assert_eq!(
            got("Object.GetCashValue(Pg.GetGuidByName('Stingray II'))"),
            [Value::Number(7500.0)]
        );
        assert_eq!(
            got("Object.GetCashValue(Pg.GetGuidByName('pmc.pony7'))"),
            [Value::Number(0.0)]
        );
        assert!(got("Object.GetCashValue(Pg.GetGuidByName('crate'))").is_empty());
        assert_eq!(got("Object.GetCashValue(nil)"), [Value::Nil]);
    }
}
