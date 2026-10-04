//! Running the game's mission scripts.
//!
//! The scripts are compiled Lua 5.1 (see `m2_assets::lua`). They are read
//! from the player's own install, rewritten for this machine's Lua loader,
//! and run in a stock Lua 5.1 VM (`mlua`, vendored). The engine libraries
//! they call (`Event`, `Object`, `Pg`, ...) are provided from Rust; any
//! the host doesn't provide yet can be stubbed, so a script runs and the
//! calls it makes are recorded instead of failing.

use std::cell::RefCell;
use std::rc::Rc;

use m2_assets::lua::Chunk;
use mlua::{ChunkMode, Function, Lua, MultiValue, Table, Value};

/// Errors from loading or running a script.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("Lua 5.0 chunks are not run")]
    Version,
    #[error(transparent)]
    Lua(#[from] mlua::Error),
}

/// A Lua VM for the game's scripts.
#[derive(Debug)]
pub struct ScriptHost {
    lua: Lua,
    /// Calls made on stubs, in order: `"Event.Create"`, `"m:SetTimer"`.
    log: Rc<RefCell<Vec<String>>>,
    /// The metatable shared by every stub.
    stub_meta: Table,
}

/// Key in a stub table holding its name.
const STUB_NAME: &str = "__m2_stub";

impl ScriptHost {
    pub fn new() -> Result<Self, ScriptError> {
        // SAFETY: loading precompiled chunks needs the unsafe constructor.
        // Only chunks from the game's own archives, rewritten by
        // `Chunk::to_host_lua51` and checked by Lua 5.1's bytecode
        // verifier on load, are loaded as binary.
        let lua = unsafe { Lua::unsafe_new() };
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let stub_meta = lua.create_table()?;
        stub_meta.set(
            "__index",
            lua.create_function(|lua, (t, key): (Table, Value)| {
                let parent: String = t.raw_get(STUB_NAME)?;
                let key = match key {
                    Value::String(s) => s.to_string_lossy(),
                    other => format!("[{}]", other.type_name()),
                };
                let child = stub(lua, &format!("{parent}.{key}"))?;
                // Remember it, so repeated lookups see the same table.
                t.raw_set(key, &child)?;
                Ok(child)
            })?,
        )?;
        let call_log = Rc::clone(&log);
        stub_meta.set(
            "__call",
            lua.create_function(move |lua, args: MultiValue| {
                let mut args = args.into_iter();
                let Some(Value::Table(callee)) = args.next() else {
                    return Ok(Value::Nil);
                };
                let name: String = callee.raw_get(STUB_NAME)?;
                // A method call passes a stub as self: `m:SetTimer` rather
                // than `m.SetTimer`.
                let receiver = args.next().and_then(|a| match a {
                    Value::Table(t) => t.raw_get::<Option<String>>(STUB_NAME).ok().flatten(),
                    _ => None,
                });
                let shown = match receiver {
                    Some(r) if name.strip_prefix(&r).is_some_and(|k| k.starts_with('.')) => {
                        format!("{r}:{}", &name[r.len() + 1..])
                    }
                    _ => name.clone(),
                };
                call_log.borrow_mut().push(shown);
                Ok(Value::Table(stub(lua, &format!("{name}()"))?))
            })?,
        )?;
        lua.set_named_registry_value("m2_stub_meta", &stub_meta)?;
        Ok(Self {
            lua,
            log,
            stub_meta,
        })
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Makes global `name` a stub: any member is another stub, and
    /// calling one records the call and returns a stub.
    pub fn stub_global(&self, name: &str) -> Result<(), ScriptError> {
        let s = stub(&self.lua, name)?;
        self.lua.globals().set(name, s)?;
        Ok(())
    }

    /// A stub named `name`, to pass to scripts (as `self`, an object, an
    /// event).
    pub fn stub(&self, name: &str) -> Result<Table, ScriptError> {
        Ok(stub(&self.lua, name)?)
    }

    /// Makes every global the host doesn't define a stub, so a script runs
    /// past libraries not written yet.
    pub fn stub_missing_globals(&self) -> Result<(), ScriptError> {
        let meta = self.lua.create_table()?;
        meta.set(
            "__index",
            self.lua
                .create_function(|lua, (g, key): (Table, Value)| match key {
                    Value::String(s) => {
                        let s = stub(lua, &s.to_string_lossy())?;
                        g.raw_set(s.raw_get::<String>(STUB_NAME)?, &s)?;
                        Ok(Value::Table(s))
                    }
                    _ => Ok(Value::Nil),
                })?,
        )?;
        self.lua.globals().set_metatable(Some(meta))?;
        Ok(())
    }

    /// Loads `chunk` as a function, named `name` in error messages.
    pub fn load(&self, chunk: &Chunk, name: &str) -> Result<Function, ScriptError> {
        let bytes = chunk
            .to_host_lua51(&format!("={name}"))
            .ok_or(ScriptError::Version)?;
        Ok(self
            .lua
            .load(&bytes[..])
            .set_name(format!("={name}"))
            .set_mode(ChunkMode::Binary)
            .into_function()?)
    }

    /// Takes the calls recorded on stubs so far.
    pub fn take_calls(&self) -> Vec<String> {
        std::mem::take(&mut self.log.borrow_mut())
    }

    pub fn is_stub(&self, value: &Value) -> bool {
        match value {
            Value::Table(t) => t
                .metatable()
                .is_some_and(|m| m.equals(&self.stub_meta).unwrap_or(false)),
            _ => false,
        }
    }
}

fn stub(lua: &Lua, name: &str) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set(STUB_NAME, name)?;
    let meta: Table = lua.named_registry_value("m2_stub_meta")?;
    t.set_metatable(Some(meta))?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// See `m2-assets`' Lua tests for the source.
    const LUA51: &[u8] = include_bytes!("../../m2-assets/tests/data/mission51.luac");

    #[test]
    fn runs_a_compiled_script_against_stubs() {
        let host = ScriptHost::new().unwrap();
        host.stub_missing_globals().unwrap();
        let chunk = Chunk::parse(LUA51).unwrap();
        host.load(&chunk, "mission")
            .unwrap()
            .call::<()>(())
            .unwrap();
        let mission: Table = host.lua().globals().get("Mission").unwrap();
        assert!(!host.is_stub(&Value::Table(mission.clone())));
        let start: Function = mission.get("Start").unwrap();
        start.call::<()>(host.stub("m").unwrap()).unwrap();
        assert_eq!(
            host.take_calls(),
            ["Objective_Add", "Hud.Show", "m:SetTimer"]
        );
    }

    #[test]
    fn script_errors_name_the_script() {
        let host = ScriptHost::new().unwrap();
        let chunk = Chunk::parse(LUA51).unwrap();
        host.load(&chunk, "mission")
            .unwrap()
            .call::<()>(())
            .unwrap();
        let start: Function = host
            .lua()
            .globals()
            .get::<Table>("Mission")
            .unwrap()
            .get("Start")
            .unwrap();
        // No stubs: Objective_Add is nil.
        let e = start.call::<()>(()).unwrap_err().to_string();
        assert!(e.contains("mission"), "{e}");
    }
}
