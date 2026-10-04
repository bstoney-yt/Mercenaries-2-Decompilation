//! Entity layers the scripts stream in and out: `Pg.LoadLayer`,
//! `Pg.UnloadLayer` and `Pg.ReloadLayer`, and the questions the layer
//! manager asks before it queues a request.
//!
//! A request is asynchronous. The script passes a callback (and values to
//! pass back to it) and waits: the layer manager counts finished requests
//! and only then runs what wanted the layers, such as a contract's start.
//! Each request becomes a [`Command::Layer`] for the game to carry out, and
//! the game reports it done with [`finished`], which calls the callback.

use std::collections::BTreeSet;

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::api::{Command, Shared};
use crate::script::safe_function;

/// What a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerOp {
    Load,
    Unload,
    Reload,
}

/// Layers the game has and which are in the world.
#[derive(Debug, Default)]
pub struct LayerState {
    /// Every layer the archives hold, lower case (`vz_state_vzacon001`).
    pub known: BTreeSet<String>,
    /// Layers in the world now, lower case.
    pub loaded: BTreeSet<String>,
    /// Requests waiting for the game: layer, what, callback and the values
    /// to pass it.
    waiting: Vec<(String, LayerOp, Option<Function>, Vec<Value>)>,
}

impl LayerState {
    pub fn is_static(name: &str) -> bool {
        name.to_ascii_lowercase().starts_with("layers_static")
    }
}

/// The layer a block holds, from its path:
/// `blocks\VZ\vz_state_vzacon001_P000_Q3.block` is `vz_state_vzacon001`.
pub fn layer_name(block_path: &str) -> String {
    let file = block_path.rsplit(['\\', '/']).next().unwrap_or(block_path);
    let stem = file.strip_suffix(".block").unwrap_or(file);
    // Drop a trailing `_P<digits>_Q<digits>` (the block's partition).
    let mut parts: Vec<&str> = stem.split('_').collect();
    let tagged = |p: &str, c: char| {
        p.len() > 1 && p.starts_with(c) && p[1..].bytes().all(|b| b.is_ascii_digit())
    };
    if parts.len() > 2 && tagged(parts[parts.len() - 1], 'Q') && tagged(parts[parts.len() - 2], 'P')
    {
        parts.truncate(parts.len() - 2);
    }
    parts.join("_").to_ascii_lowercase()
}

/// Reports the request for `name` (any case) done, and calls the callbacks
/// waiting on it as the layer manager expects: the values of the request's
/// table, the layer's name, an unused value, and whether it worked.
/// Returns the callbacks that failed.
pub fn finished(world: &Shared, name: &str, ok: bool) -> Vec<mlua::Error> {
    let name = name.to_ascii_lowercase();
    let done: Vec<_> = {
        let mut w = world.borrow_mut();
        let (done, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut w.layers.waiting)
            .into_iter()
            .partition(|r| r.0 == name);
        w.layers.waiting = keep;
        for (_, op, _, _) in &done {
            if ok {
                match op {
                    LayerOp::Load | LayerOp::Reload => w.layers.loaded.insert(name.clone()),
                    LayerOp::Unload => w.layers.loaded.remove(&name),
                };
            }
        }
        done
    };
    let mut errors = Vec::new();
    for (_, _, callback, mut args) in done {
        let Some(f) = callback else { continue };
        // The request's values end with the layer's name as a Lua string;
        // then a value the layer manager never reads, and the result.
        args.push(Value::Nil);
        args.push(Value::Boolean(ok));
        if let Err(e) = f.call::<()>(MultiValue::from_vec(args)) {
            errors.push(e);
        }
    }
    errors
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// Installs the layer functions on `Pg`.
pub fn install(lua: &Lua, world: &Shared) -> mlua::Result<()> {
    let pg: Table = match lua.globals().raw_get::<Value>("Pg")? {
        Value::Table(t) if t.metatable().is_none() => t,
        _ => {
            let t = lua.create_table()?;
            lua.globals().raw_set("Pg", &t)?;
            t
        }
    };
    for (name, op) in [
        ("LoadLayer", LayerOp::Load),
        ("UnloadLayer", LayerOp::Unload),
        ("ReloadLayer", LayerOp::Reload),
    ] {
        let w = std::rc::Rc::clone(world);
        pg.set(
            name,
            safe_function(lua, move |lua, args| {
                let args: Vec<Value> = args.into_iter().collect();
                let Some(layer) = text(args.first()).map(|s| s.to_ascii_lowercase()) else {
                    return Ok(MultiValue::from_vec(vec![Value::Boolean(false)]));
                };
                // The callback is the first function; the values after it
                // go back to it.
                let at = args.iter().position(|v| matches!(v, Value::Function(_)));
                let (callback, data) = match at {
                    Some(i) => {
                        let Value::Function(f) = &args[i] else {
                            unreachable!()
                        };
                        // The values come as a table (`{"Load"}`).
                        let data = match args.get(i + 1) {
                            Some(Value::Table(t)) => {
                                t.sequence_values().collect::<mlua::Result<Vec<Value>>>()?
                            }
                            _ => Vec::new(),
                        };
                        (Some(f.clone()), data)
                    }
                    None => (None, Vec::new()),
                };
                let mut data = data;
                data.push(Value::String(lua.create_string(&layer)?));
                let mut w = w.borrow_mut();
                w.layers.waiting.push((layer.clone(), op, callback, data));
                w.commands.push(Command::Layer { op, name: layer });
                Ok(MultiValue::from_vec(vec![Value::Boolean(true)]))
            })?,
        )?;
    }
    let w = std::rc::Rc::clone(world);
    pg.set(
        "AssetExists",
        safe_function(lua, move |_, args| {
            let name = text(args.front()).unwrap_or_default().to_ascii_lowercase();
            let kind = text(args.get(1)).unwrap_or_default();
            // Layers (`Pg.AssetExists(name, "layer")`) are checked against
            // the archives; other assets are taken on trust.
            let exists =
                !kind.eq_ignore_ascii_case("layer") || w.borrow().layers.known.contains(&name);
            Ok(MultiValue::from_vec(vec![Value::Boolean(exists)]))
        })?,
    )?;
    pg.set(
        "IsStaticLayer",
        safe_function(lua, |_, args| {
            let name = text(args.front()).unwrap_or_default();
            Ok(MultiValue::from_vec(vec![Value::Boolean(
                LayerState::is_static(&name),
            )]))
        })?,
    )?;
    pg.set(
        "GetUnloadingStaticLayers",
        safe_function(lua, |lua, _| {
            Ok(MultiValue::from_vec(vec![Value::Table(
                lua.create_table()?,
            )]))
        })?,
    )?;
    pg.set(
        "UnloadingStaticLayers",
        safe_function(lua, |_, _| {
            Ok(MultiValue::from_vec(vec![Value::Boolean(false)]))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{self, ScriptWorld};
    use crate::script::ScriptHost;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn names_come_from_block_paths() {
        assert_eq!(
            layer_name(r"blocks\VZ\vz_state_vzacon001_P000_Q3.block"),
            "vz_state_vzacon001"
        );
        assert_eq!(layer_name("Vz_State_Pmc_Act1"), "vz_state_pmc_act1");
        assert!(LayerState::is_static("Layers_Static"));
    }

    #[test]
    fn requests_wait_for_the_game_then_call_back() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        world
            .borrow_mut()
            .layers
            .known
            .insert("vz_state_vzacon001".into());
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                done = {}
                assert(Pg.AssetExists("Vz_State_VzaCon001", "layer"))
                assert(not Pg.AssetExists("vz_state_nothing", "layer"))
                assert(Pg.AssetExists("some_model"))
                -- As MrxLayerManager calls it: name, dynamic, callback,
                -- its values, needs a loading screen.
                local ok = Pg.LoadLayer("vz_state_vzacon001", true, function(op, name, _, ok)
                    done[#done + 1] = op .. " " .. name .. " " .. tostring(ok)
                end, {"Load"}, false)
                assert(ok)
                "#,
            )
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [Command::Layer {
                op: LayerOp::Load,
                name: "vz_state_vzacon001".into()
            }]
        );
        assert!(finished(&world, "VZ_STATE_VZACON001", true).is_empty());
        let done: Vec<String> = host.lua().load("return done").eval().unwrap();
        assert_eq!(done, ["Load vz_state_vzacon001 true"]);
        assert!(world.borrow().layers.loaded.contains("vz_state_vzacon001"));
    }
}
