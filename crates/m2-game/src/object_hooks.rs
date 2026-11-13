//! The engine's calls into object scripts: what it tells the script of an
//! object (its `ObjectScript`) besides that the object came in or went.
//!
//! **From the 360 code.** One routine (0x82309690) makes every such call.
//! It is given the object's `ObjectScript` record (module, data), the
//! hook's name and the arguments already pushed; with no record or no
//! module it does nothing, and otherwise it pushes the record's second
//! field after the arguments and calls the module's function of that
//! name with one argument more. A module without that function is passed
//! over without an error (0x823094F0 looks the name up first). So every
//! hook's last argument is the record's data, as `OnActivate`'s is.
//!
//! The routine has eleven callers, one per hook. The two this module
//! dispatches:
//!
//! ```text
//! OnDeath(guid, data)                    0x82289F40, the death handler
//! OnStateChange(guid, machine, state, data)
//!                                        0x82231380, a state machine's
//!                                        change of state
//! ```
//!
//! `OnDeath` is called once the death's own spawns are made. An object
//! without a record of its own has another object's tried
//! (0x823F7B88 of its guid); which object that is isn't established, and
//! it is not reproduced here.
//!
//! `OnStateChange` is called only when the state really changes, after
//! the old state's leaving actions and the new state's entering actions
//! have run; `machine` and `state` are name hashes, the kind
//! `String.GetHash` gives (the shooting gallery targets turn theirs into
//! text, the trees compare theirs with `String.GetHash("FireDebrisState")`).
//!
//! The other nine (`OnActivate`, `OnDeactivate`, `Use`, `UnUse`,
//! `QueryActiveUse`, `OnUse`, `OnDesignate`, `OnTimer`,
//! `DisableGrappleTriggered`) are not dispatched from here.
//!
//! **Invented:** nothing. An entity placed in a layer has its own record;
//! an object spawned in play has its template's
//! ([`crate::api::ScriptWorld::note_script`]).

use mlua::{Function, IntoLua, Lua, MultiValue, Table, Value};

use crate::events::guid_value;
use crate::mission::Entity;

/// `OnDeath`, the hook of an object that died.
pub const ON_DEATH: &str = "OnDeath";
/// `OnStateChange`, the hook of an object one of whose state machines
/// changed state.
pub const ON_STATE_CHANGE: &str = "OnStateChange";

/// The `ObjectScript` record of the entity `guid` among `entities`: its
/// module's hash and its data.
pub fn record<'a>(
    entities: impl IntoIterator<Item = &'a Entity>,
    guid: u32,
) -> Option<(u32, Option<u32>)> {
    entities
        .into_iter()
        .find(|e| e.guid == guid)
        .and_then(|e| Some((e.script?, e.script_data)))
}

/// Calls `hook` of the module `record` names, among the modules the port
/// runs (`modules`: name and hash), with `args` and then the record's
/// data. `Ok(false)` when nothing was there to call: no record, a module
/// the port doesn't run, or a module without that function.
pub fn call(
    lua: &Lua,
    modules: &[(&str, u32)],
    record: Option<(u32, Option<u32>)>,
    hook: &str,
    args: Vec<Value>,
) -> mlua::Result<bool> {
    let Some((hash, data)) = record else {
        return Ok(false);
    };
    let Some((module, _)) = modules.iter().find(|(_, h)| *h == hash) else {
        return Ok(false);
    };
    let Ok(table) = lua.globals().get::<Table>(*module) else {
        return Ok(false);
    };
    let Ok(function) = table.get::<Function>(hook) else {
        return Ok(false);
    };
    let mut args = args;
    // As `OnActivate`'s is handed over: a number (an integer where the
    // build's Lua integers hold it).
    args.push(data.into_lua(lua)?);
    function.call::<()>(MultiValue::from_vec(args))?;
    Ok(true)
}

/// `OnDeath`'s arguments before the data: the dead object.
pub fn death_args(guid: u32) -> Vec<Value> {
    vec![guid_value(guid)]
}

/// `OnStateChange`'s arguments before the data: the object, the state
/// machine's name hash and the new state's.
pub fn state_change_args(guid: u32, machine: u32, state: u32) -> Vec<Value> {
    vec![guid_value(guid), guid_value(machine), guid_value(state)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULES: [(&str, u32); 2] = [("Mine", 0xcf21_0c94), ("Target", 0xccfb_6897)];

    fn lua() -> Lua {
        let lua = Lua::new();
        lua.load(
            r#"
            calls = {}
            Mine = { OnDeath = function(...) calls[#calls + 1] = { "death", ... } end }
            Target = { OnStateChange = function(...) calls[#calls + 1] = { "state", ... } end }
            function seen(i, j) return calls[i] and calls[i][j] end
            "#,
        )
        .exec()
        .unwrap();
        lua
    }

    fn seen(lua: &Lua, call: usize, arg: usize) -> Value {
        let f: Function = lua.globals().get("seen").unwrap();
        f.call((call, arg)).unwrap()
    }

    #[test]
    fn death_passes_the_object_then_the_records_data() {
        let lua = lua();
        let record = Some((0xcf21_0c94, Some(7)));
        assert!(call(&lua, &MODULES, record, ON_DEATH, death_args(0x1234)).unwrap());
        assert_eq!(seen(&lua, 1, 2), guid_value(0x1234));
        assert_eq!(crate::events::as_guid(&seen(&lua, 1, 3)), Some(7));
        assert_eq!(seen(&lua, 1, 4), Value::Nil);
    }

    #[test]
    fn a_state_change_passes_object_machine_state_and_data() {
        let lua = lua();
        let record = Some((0xccfb_6897, None));
        let args = state_change_args(0x55, 0x1111_2222, 0x7687_df41);
        assert!(call(&lua, &MODULES, record, ON_STATE_CHANGE, args).unwrap());
        assert_eq!(seen(&lua, 1, 2), guid_value(0x55));
        assert_eq!(seen(&lua, 1, 3), guid_value(0x1111_2222));
        assert_eq!(seen(&lua, 1, 4), guid_value(0x7687_df41));
        // No data in the record: nil, still passed last.
        assert_eq!(seen(&lua, 1, 5), Value::Nil);
    }

    #[test]
    fn the_state_hash_equals_what_the_scripts_hash() {
        let lua = lua();
        let hash = crate::script_hash::get_hash_native(&[Value::String(
            lua.create_string("FireDebrisState").unwrap(),
        )]);
        let h = crate::events::as_guid(&hash[0]).unwrap();
        assert_eq!(state_change_args(1, 2, h)[2], hash[0]);
    }

    #[test]
    fn nothing_is_called_without_a_record_a_run_module_or_the_function() {
        let lua = lua();
        // No record.
        assert!(!call(&lua, &MODULES, None, ON_DEATH, death_args(1)).unwrap());
        // A module the port doesn't run.
        let other = Some((0xdead_beef, None));
        assert!(!call(&lua, &MODULES, other, ON_DEATH, death_args(1)).unwrap());
        // A module without the hook, as the engine passes it over.
        let target = Some((0xccfb_6897, None));
        assert!(!call(&lua, &MODULES, target, ON_DEATH, death_args(1)).unwrap());
        assert_eq!(seen(&lua, 1, 1), Value::Nil);
    }

    #[test]
    fn a_hooks_error_comes_back() {
        let lua = Lua::new();
        lua.load("Mine = { OnDeath = function() error('boom') end }")
            .exec()
            .unwrap();
        let record = Some((0xcf21_0c94, None));
        let err = call(&lua, &MODULES, record, ON_DEATH, death_args(1)).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn an_entitys_record_is_its_script_and_data() {
        let mut e = Entity {
            guid: 9,
            name: "mine".into(),
            at: [0.0; 3],
            yaw: 0.0,
            region: None,
            labels: Vec::new(),
            script: None,
            script_data: Some(4),
            faction: None,
            path: Vec::new(),
            template: None,
            alarm: false,
            association: None,
            crew: Vec::new(),
        };
        // No script, no record, whatever its data.
        assert_eq!(record([&e], 9), None);
        e.script = Some(0xcf21_0c94);
        assert_eq!(record([&e], 9), Some((0xcf21_0c94, Some(4))));
        assert_eq!(record([&e], 10), None);
    }
}
