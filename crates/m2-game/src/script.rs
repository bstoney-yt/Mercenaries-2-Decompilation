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

use crate::library::Library;
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
    /// Modules loaded so far, in order, with the script each resolved to.
    modules: Rc<RefCell<Vec<LoadedModule>>>,
    /// Modules in the order their chunks finished running: a module after
    /// everything it imports, the order `Init` runs in.
    finished: Rc<RefCell<Vec<String>>>,
}

/// A module name as loaded, and the script hash it resolved to.
pub type LoadedModule = (String, Option<u32>);

thread_local! {
    /// Whether stub calls are logged with their arguments.
    static STUB_ARGS: bool = std::env::var_os("M2_STUB_ARGS").is_some();
}

/// Whether stub calls are logged with their arguments (`M2_STUB_ARGS`).
pub fn stub_args() -> bool {
    STUB_ARGS.with(|on| *on)
}

/// A stub call's argument as the log shows it: numbers, short strings,
/// stubs by name, other tables as `{}`.
pub(crate) fn show_arg(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_owned(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => format!("{n:.2}"),
        Value::String(s) => {
            let s = s.to_string_lossy();
            let cut: String = s.chars().take(40).collect();
            format!("{cut:?}")
        }
        Value::Table(t) => t
            .raw_get::<Option<String>>(STUB_NAME)
            .ok()
            .flatten()
            .unwrap_or_else(|| "{}".to_owned()),
        other => other.type_name().to_owned(),
    }
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
        let wrap: Function = lua
            .load(
                r#"
                local function check(ok, ...)
                    if ok then return ... end
                    error((...), 2)
                end
                return function(raw)
                    return function(...) return check(raw(...)) end
                end
                "#,
            )
            .set_name("=m2-wrap")
            .set_mode(ChunkMode::Text)
            .eval()?;
        lua.set_named_registry_value("m2_wrap", wrap)?;
        let stub_meta = lua.create_table()?;
        stub_meta.set(
            "__index",
            safe_function(&lua, |lua, args| {
                let (Some(Value::Table(t)), key) = (args.front().cloned(), args.get(1)) else {
                    return Ok(MultiValue::new());
                };
                let parent: String = t.raw_get(STUB_NAME)?;
                let key = match key {
                    Some(Value::String(s)) => s.to_string_lossy(),
                    other => format!("[{}]", other.map_or("nil", Value::type_name)),
                };
                let child = stub(lua, &format!("{parent}.{key}"))?;
                // Remember it, so repeated lookups see the same table.
                t.raw_set(key, &child)?;
                Ok(MultiValue::from_vec(vec![Value::Table(child)]))
            })?,
        )?;
        let call_log = Rc::clone(&log);
        stub_meta.set(
            "__call",
            safe_function(&lua, move |lua, args| {
                let mut args = args.into_iter();
                let Some(Value::Table(callee)) = args.next() else {
                    return Ok(MultiValue::new());
                };
                let name: String = callee.raw_get(STUB_NAME)?;
                // A method call passes a stub as self: `m:SetTimer` rather
                // than `m.SetTimer`.
                let first = args.next();
                let receiver = first.as_ref().and_then(|a| match a {
                    Value::Table(t) => t.raw_get::<Option<String>>(STUB_NAME).ok().flatten(),
                    _ => None,
                });
                let method = receiver.as_ref().is_some_and(|r| {
                    name.strip_prefix(r.as_str())
                        .is_some_and(|k| k.starts_with('.'))
                });
                let mut shown = match &receiver {
                    Some(r) if method => format!("{r}:{}", &name[r.len() + 1..]),
                    _ => name.clone(),
                };
                // With `M2_STUB_ARGS` set, the arguments too (to learn what
                // the scripts ask of a call before it is written).
                if STUB_ARGS.with(|on| *on) {
                    let rest = (!method).then_some(first).flatten().into_iter().chain(args);
                    let shown_args: Vec<String> = rest.map(|v| show_arg(&v)).collect();
                    shown = format!("{shown}({})", shown_args.join(", "));
                }
                call_log.borrow_mut().push(shown);
                Ok(MultiValue::from_vec(vec![Value::Table(stub(
                    lua,
                    &format!("{name}()"),
                )?)]))
            })?,
        )?;
        // Stubs stand in for numbers too (`nHeight = Graphics.GetHeight()`):
        // in arithmetic a stub is 0, and joined to a string it is its name.
        let arithmetic: Table = lua
            .load(
                r#"
                local function n(v) return type(v) == "number" and v or 0 end
                local function s(v)
                    if type(v) == "table" then return rawget(v, "__m2_stub") or "" end
                    return v
                end
                return {
                    __add = function(a, b) return n(a) + n(b) end,
                    __sub = function(a, b) return n(a) - n(b) end,
                    __mul = function(a, b) return n(a) * n(b) end,
                    __div = function(a, b) return n(a) / n(b) end,
                    __mod = function(a, b) return n(a) % n(b) end,
                    __pow = function(a, b) return n(a) ^ n(b) end,
                    __unm = function() return 0 end,
                    __concat = function(a, b) return s(a) .. s(b) end,
                    __lt = function() return false end,
                    __le = function() return true end,
                }
                "#,
            )
            .set_name("=m2-stub-arithmetic")
            .set_mode(ChunkMode::Text)
            .eval()?;
        for pair in arithmetic.pairs::<String, Function>() {
            let (k, v) = pair?;
            stub_meta.set(k, v)?;
        }
        lua.set_named_registry_value("m2_stub_meta", &stub_meta)?;
        Ok(Self {
            lua,
            log,
            stub_meta,
            modules: Rc::default(),
            finished: Rc::default(),
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
    /// past libraries not written yet; see `default_for` for the globals
    /// that get plain values instead.
    pub fn stub_missing_globals(&self) -> Result<(), ScriptError> {
        let meta = self.lua.create_table()?;
        meta.set(
            "__index",
            safe_function(&self.lua, |lua, args| match (args.front(), args.get(1)) {
                (Some(Value::Table(g)), Some(Value::String(s))) => {
                    let name = s.to_string_lossy();
                    // `_bBootstrapComplete` and the like: a script's own
                    // state, nil until it sets it (and again after it sets
                    // it to nil).
                    if name.starts_with('_') {
                        return Ok(MultiValue::from_vec(vec![Value::Nil]));
                    }
                    let value = match default_for(lua, &name)? {
                        Some(v) => v,
                        None => Value::Table(stub(lua, &name)?),
                    };
                    g.raw_set(name, &value)?;
                    Ok(MultiValue::from_vec(vec![value]))
                }
                _ => Ok(MultiValue::new()),
            })?,
        )?;
        self.lua.globals().set_metatable(Some(meta))?;
        Ok(())
    }

    /// Makes the members `libraries` (host tables such as `Pg`) don't
    /// define functions that record the call, as stubs do, and return
    /// nothing: an engine call not written yet answers nil rather than
    /// failing the script.
    pub fn complete_libraries(&self, libraries: &[&str]) -> Result<(), ScriptError> {
        for &library in libraries {
            let Value::Table(t) = self.lua.globals().raw_get::<Value>(library)? else {
                continue;
            };
            if t.metatable().is_some() {
                continue;
            }
            let meta = self.lua.create_table()?;
            let log = Rc::clone(&self.log);
            let library = library.to_owned();
            meta.set(
                "__index",
                safe_function(&self.lua, move |lua, args| {
                    let (Some(Value::Table(t)), Some(Value::String(k))) =
                        (args.front(), args.get(1))
                    else {
                        return Ok(MultiValue::new());
                    };
                    let name = format!("{library}.{}", k.to_string_lossy());
                    let log = Rc::clone(&log);
                    let f = safe_function(lua, move |_, args| {
                        let mut shown = name.clone();
                        if STUB_ARGS.with(|on| *on) {
                            let args: Vec<String> = args.iter().map(show_arg).collect();
                            shown = format!("{shown}({})", args.join(", "));
                        }
                        log.borrow_mut().push(shown);
                        Ok(MultiValue::new())
                    })?;
                    t.raw_set(k.clone(), &f)?;
                    Ok(MultiValue::from_vec(vec![Value::Function(f)]))
                })?,
            )?;
            t.set_metatable(Some(meta))?;
        }
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
    /// Provides `import`, `inherit`, `dynamic_import` and `dynamic_remove`
    /// over `library`. A module runs once, in its own environment (which
    /// falls back to the globals), and that environment is the module (and
    /// a global of that name, unless one is already defined):
    /// `import("MrxUtil")` binds it as `MrxUtil` in the caller's
    /// environment, and `inherit("MrxTask")` also makes the caller's
    /// environment fall back to it, so the script extends that class. A
    /// module no script resolves to is empty, or with `stub_unresolved`
    /// answers every missing member with a stub.
    pub fn install_library(
        &self,
        library: Rc<Library>,
        stub_unresolved: bool,
    ) -> Result<(), ScriptError> {
        let cache = self.lua.create_table()?;
        let modules = Rc::clone(&self.modules);
        let finished = Rc::clone(&self.finished);
        let module = safe_function(&self.lua, move |lua, args| {
            let Some(Value::String(name)) = args.front() else {
                return Err(mlua::Error::runtime("module name must be a string"));
            };
            let name = name.to_string_lossy();
            let key = name.to_ascii_lowercase();
            if let Some(env) = cache.raw_get::<Option<Table>>(key.as_str())? {
                return Ok(MultiValue::from_vec(vec![Value::Table(env)]));
            }
            // A script is one module: one already loaded under another
            // name is a wrong match for this one.
            let resolved = library.resolve(&name).map(|(s, _, _)| s).filter(|s| {
                !modules.borrow().iter().any(|(other, hash)| {
                    *hash == Some(s.hash) && !other.eq_ignore_ascii_case(&name)
                })
            });
            modules
                .borrow_mut()
                .push((name.clone(), resolved.map(|s| s.hash)));
            let env = lua.create_table()?;
            let meta = lua.create_table()?;
            if resolved.is_none() && stub_unresolved {
                let prefix = name.clone();
                meta.set(
                    "__index",
                    safe_function(lua, move |lua, args| {
                        let Some(Value::String(k)) = args.get(1) else {
                            return Ok(MultiValue::new());
                        };
                        let k = k.to_string_lossy();
                        let v = match lua.globals().get::<Value>(k.as_str())? {
                            Value::Nil => Value::Table(stub(lua, &format!("{prefix}.{k}"))?),
                            v => v,
                        };
                        Ok(MultiValue::from_vec(vec![v]))
                    })?,
                )?;
            } else {
                meta.set("__index", lua.globals())?;
            }
            env.set_metatable(Some(meta))?;
            // Each module sees itself as `_THIS` (MrxTask.CreateChild
            // makes children with `_THIS:Create()`).
            env.raw_set("_THIS", &env)?;
            // Cached before it runs, so modules loading each other finish.
            cache.raw_set(key.as_str(), &env)?;
            // Loaded modules are globals too: scripts use some they never
            // import themselves (MrxMissionFlow reads WifMissionData).
            let globals = lua.globals();
            let free = match globals.raw_get::<Value>(name.as_str())? {
                Value::Nil => true,
                Value::Table(t) => t.raw_get::<Value>(STUB_NAME)? != Value::Nil,
                _ => false,
            };
            if free {
                globals.raw_set(name.as_str(), &env)?;
            }
            if let Some(script) = resolved {
                let source = format!("={name}");
                let bytes = script
                    .chunk
                    .to_host_lua51(&source)
                    .ok_or_else(|| mlua::Error::runtime(format!("{name}: Lua 5.0 chunk")))?;
                lua.load(&bytes[..])
                    .set_name(source)
                    .set_mode(ChunkMode::Binary)
                    .set_environment(env.clone())
                    .exec()?;
                finished.borrow_mut().push(name.clone());
            }
            Ok(MultiValue::from_vec(vec![Value::Table(env)]))
        })?;
        self.lua.globals().set("__m2_module", module)?;
        let notes = Rc::clone(&self.log);
        self.lua.globals().set(
            "__m2_note",
            safe_function(&self.lua, move |_, args| {
                if let Some(Value::String(s)) = args.front() {
                    notes
                        .borrow_mut()
                        .push(format!("note: {}", s.to_string_lossy()));
                }
                Ok(MultiValue::new())
            })?,
        )?;
        self.lua
            .load(
                r#"
                local module = __m2_module
                -- The environment of import's caller; the globals when the
                -- host calls import itself. (Under pcall, level 1 is pcall,
                -- 2 this function, 3 import and 4 its caller.)
                local function caller()
                    local ok, env = pcall(getfenv, 4)
                    if ok and type(env) == "table" then return env end
                    return _G
                end
                local function bind(env, name)
                    local m = module(name)
                    rawset(env, name, m)
                    return m
                end
                -- Whether t's fallback chain reaches target.
                local function reaches(t, target)
                    for _ = 1, 64 do
                        if t == target then return true end
                        local meta = getmetatable(t)
                        if type(meta) ~= "table" or type(meta.__index) ~= "table" then
                            return false
                        end
                        t = meta.__index
                    end
                    return true
                end
                function import(name) return bind(caller(), name) end
                -- dynamic_import(name, callback, args) loads in the
                -- background in the game: the callback runs later, with
                -- the args unpacked and then the module, when the host
                -- runs the deferred calls.
                __m2_deferred = {}
                function dynamic_import(name, callback, args)
                    local m = bind(caller(), name)
                    if type(callback) == "function" then
                        local a = {}
                        for i, v in ipairs(args or {}) do a[i] = v end
                        a[#a + 1] = m
                        local q = __m2_deferred
                        q[#q + 1] = { callback, a }
                    end
                    return m
                end
                function __m2_run_deferred()
                    local q = __m2_deferred
                    __m2_deferred = {}
                    local errors = {}
                    for _, d in ipairs(q) do
                        local ok, e = pcall(d[1], unpack(d[2]))
                        if not ok then errors[#errors + 1] = tostring(e) end
                    end
                    return #q, errors
                end
                function dynamic_remove(name) end
                function inherit(name)
                    local env = caller()
                    local base = bind(env, name)
                    -- A wrongly resolved module could make a class its own
                    -- ancestor; keep the globals as the fallback then.
                    if env ~= _G and not reaches(base, env) then
                        setmetatable(env, { __index = base })
                    else
                        __m2_note("inherit " .. tostring(name) .. " would loop")
                    end
                    return base
                end
                "#,
            )
            .set_name("=m2-modules")
            .set_mode(ChunkMode::Text)
            .exec()?;
        Ok(())
    }

    /// Runs the callbacks of `dynamic_import`s made since the last call
    /// (the modules are loaded; the game would have streamed them).
    /// Returns how many ran and the errors of those that failed. Call
    /// once a tick.
    pub fn run_deferred(&self) -> Result<(usize, Vec<String>), ScriptError> {
        let Ok(run) = self.lua.globals().raw_get::<Function>("__m2_run_deferred") else {
            return Ok((0, Vec::new()));
        };
        Ok(run.call::<(usize, Vec<String>)>(())?)
    }

    /// Calls `Init` on every module loaded so far that defines one, as the
    /// game's bootstraps do before play: many modules set up their tables
    /// there. A module's `Init` runs after those of the modules it
    /// imports (`MrxPmc.Init` builds widgets with what `MrxGui.Init` set
    /// up). Returns those that failed.
    pub fn init_modules(&self) -> Result<Vec<String>, ScriptError> {
        let mut failed = Vec::new();
        let globals = self.lua.globals();
        // An Init may import more modules, which append to the list (and
        // must not find it borrowed): snapshot it, and go round again for
        // the ones that joined.
        let mut done = 0;
        loop {
            let order = self.finished.borrow().clone();
            if done >= order.len() {
                break;
            }
            for name in &order[done..] {
                let Ok(Value::Table(m)) = globals.raw_get::<Value>(name.as_str()) else {
                    continue;
                };
                if let Ok(Value::Function(init)) = m.raw_get::<Value>("Init")
                    && let Err(e) = init.call::<()>(())
                {
                    failed.push(format!("{name}: {}", tidy(&e.to_string())));
                }
            }
            done = order.len();
        }
        Ok(failed)
    }

    /// Runs script `hash` from `library` as an entry point: in its own
    /// environment, like a module, returning that environment.
    pub fn run_script(&self, library: &Library, hash: u32) -> Result<Table, ScriptError> {
        let script = library
            .by_hash(hash)
            .ok_or_else(|| mlua::Error::runtime(format!("no script {hash:08x}")))?;
        let env = self.lua.create_table()?;
        let meta = self.lua.create_table()?;
        meta.set("__index", self.lua.globals())?;
        env.set_metatable(Some(meta))?;
        env.raw_set("_THIS", &env)?;
        let f = self.load(&script.chunk, &format!("{hash:08x}"))?;
        f.set_environment(env.clone())?;
        f.call::<()>(())?;
        Ok(env)
    }

    /// Modules loaded so far, in order, with the script each resolved to.
    pub fn modules(&self) -> Vec<LoadedModule> {
        self.modules.borrow().clone()
    }
}

/// A value for a global the scripts read but nothing defines, from the
/// type prefix of their naming convention (`nCount`, `sName`, `bActive`,
/// `tList`): 0, "", false or an empty table. The screen size is the
/// alpha's default window. `None` for other names, which become stubs.
fn default_for(lua: &Lua, name: &str) -> mlua::Result<Option<Value>> {
    let mut chars = name.chars();
    let (Some(prefix), Some(next)) = (chars.next(), chars.next()) else {
        return Ok(None);
    };
    if !next.is_ascii_uppercase() {
        return Ok(None);
    }
    Ok(match (prefix, name) {
        (_, "nScreenWidth") => Some(Value::Number(1600.0)),
        (_, "nScreenHeight") => Some(Value::Number(900.0)),
        ('n', _) => Some(Value::Number(0.0)),
        ('s', _) => Some(Value::String(lua.create_string("")?)),
        ('b', _) => Some(Value::Boolean(false)),
        ('t', _) => Some(Value::Table(lua.create_table()?)),
        // An object a script keeps (`oSystemDialogBoxFlash`): nil until
        // made, as scripts test it (`if MrxGuiDialogBox.oSystemDialogBoxFlash
        // then`, read through the module to here).
        ('o', _) => Some(Value::Nil),
        _ => None,
    })
}

/// Drops the "runtime error: " each nested call adds to a message.
fn tidy(message: &str) -> &str {
    let mut m = message;
    while let Some(rest) = m.strip_prefix("runtime error: ") {
        m = rest;
    }
    m
}

/// A function scripts can call that runs `f`.
///
/// A Rust callback must never raise a Lua error itself: Lua 5.1 built as C
/// raises errors with `longjmp`, which on Windows unwinds through the Rust
/// frames between and aborts the process. So the Rust side returns
/// `true, results...` or `false, message`, and a small Lua wrapper raises
/// the error from Lua.
pub fn safe_function<F>(lua: &Lua, f: F) -> mlua::Result<Function>
where
    F: Fn(&Lua, MultiValue) -> mlua::Result<MultiValue> + 'static,
{
    let raw = lua.create_function(move |lua, args: MultiValue| {
        Ok(match f(lua, args) {
            Ok(mut results) => {
                results.push_front(Value::Boolean(true));
                results
            }
            Err(e) => MultiValue::from_vec(vec![
                Value::Boolean(false),
                Value::String(lua.create_string(tidy(&e.to_string()))?),
            ]),
        })
    })?;
    let wrap: Function = lua.named_registry_value("m2_wrap")?;
    wrap.call(raw)
}

/// The name of a stub (`"HealthCounter.Update"`), `None` for any other
/// value.
pub fn stub_name(value: &Value) -> Option<String> {
    match value {
        Value::Table(t) => t.raw_get::<Option<String>>(STUB_NAME).ok().flatten(),
        _ => None,
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
    fn unset_globals_are_stubs_unless_read_raw() {
        let host = ScriptHost::new().unwrap();
        host.stub_missing_globals().unwrap();
        // Test drivers read their settings raw (`DRIVE_UNTIL`).
        let (plain, raw): (String, String) = host
            .lua()
            .load("return type(DRIVE_SOMEWHERE), type(rawget(_G, 'DRIVE_UNTIL'))")
            .eval()
            .unwrap();
        assert_eq!((plain.as_str(), raw.as_str()), ("table", "nil"));
    }

    #[test]
    fn modules_import_and_inherit_each_other() {
        let host = ScriptHost::new().unwrap();
        let library = Rc::new(crate::library::tests::library());
        host.install_library(Rc::clone(&library), false).unwrap();
        let user = host.run_script(&library, 4).unwrap();
        assert_eq!(user.get::<String>("Result").unwrap(), "hi there");
        let contract = host.run_script(&library, 3).unwrap();
        let task = host.lua().create_table().unwrap();
        task.set("n", 5).unwrap();
        // Its own Activated calls the base's: 5 * 2 + 1.
        let activated: Function = contract.get("Activated").unwrap();
        assert_eq!(activated.call::<i64>(&task).unwrap(), 11);
        // Inherited, not defined by the contract.
        let describe: Function = contract.get("Describe").unwrap();
        assert_eq!(describe.call::<String>(&task).unwrap(), "task");
        assert_eq!(
            host.modules(),
            [("Util".to_owned(), Some(1)), ("Task".to_owned(), Some(2))]
        );
        // Loaded modules are globals too, for scripts that never import
        // them.
        let greet: String = host.lua().load("return Util.Greet('you')").eval().unwrap();
        assert_eq!(greet, "hi you");
    }

    #[test]
    fn dynamic_imports_call_back_later_and_modules_init() {
        let host = ScriptHost::new().unwrap();
        let library = Rc::new(crate::library::tests::library());
        host.install_library(Rc::clone(&library), false).unwrap();
        host.lua()
            .load(
                r#"
                got = nil
                local m = dynamic_import("Util", function(a, b, mod)
                    got = a .. b
                    last_module = mod
                end, {"x", "y"})
                assert(m.Greet ~= nil and got == nil)
                Util.Init = function() Util.ready = true end
                "#,
            )
            .exec()
            .unwrap();
        assert_eq!(host.run_deferred().unwrap(), (1, Vec::new()));
        let got: String = host.lua().globals().get("got").unwrap();
        assert_eq!(got, "xy");
        let (this, module): (bool, bool) = host
            .lua()
            .load("return Util._THIS == Util, last_module == Util")
            .eval()
            .unwrap();
        assert_eq!((this, module), (true, true));
        assert!(host.init_modules().unwrap().is_empty());
        let ready: bool = host.lua().load("return Util.ready").eval().unwrap();
        assert!(ready);
    }

    #[test]
    fn modules_init_after_the_modules_they_import() {
        let host = ScriptHost::new().unwrap();
        let library = Rc::new(crate::library::tests::library());
        host.install_library(library, false).unwrap();
        // Task imports Util: Task starts loading first, Util finishes
        // first, and Init follows the finishing order.
        host.lua()
            .load(
                r#"
                import("Task")
                order = {}
                Util.Init = function() order[#order + 1] = "Util" end
                -- An Init may import more: those run their Init too.
                Task.Init = function()
                    order[#order + 1] = "Task"
                    import("There").Init = function() order[#order + 1] = "There" end
                end
                "#,
            )
            .exec()
            .unwrap();
        assert_eq!(
            host.modules(),
            [("Task".to_owned(), Some(2)), ("Util".to_owned(), Some(1))]
        );
        assert!(host.init_modules().unwrap().is_empty());
        let order: Vec<String> = host
            .lua()
            .load("return order")
            .eval::<Table>()
            .unwrap()
            .sequence_values()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(order, ["Util", "Task", "There"]);
    }

    #[test]
    fn a_script_is_only_one_module() {
        let host = ScriptHost::new().unwrap();
        let library = Rc::new(crate::library::tests::library());
        host.install_library(library, false).unwrap();
        // Neither name is a module; both fall back to the script naming
        // them (user.lua), which can only be the first.
        host.lua()
            .load("import('There'); import('Greet')")
            .exec()
            .unwrap();
        assert_eq!(
            host.modules(),
            [
                ("There".to_owned(), Some(4)),
                ("Util".to_owned(), Some(1)),
                ("Greet".to_owned(), None)
            ]
        );
    }

    #[test]
    fn unresolved_modules_can_be_stubs() {
        let host = ScriptHost::new().unwrap();
        host.install_library(Rc::new(Library::default()), true)
            .unwrap();
        host.lua()
            .load("import('Hud'); Hud.Show('x'); assert(tostring(1) == '1')")
            .exec()
            .unwrap();
        assert_eq!(host.take_calls(), ["Hud.Show"]);
        assert_eq!(host.modules(), [("Hud".to_owned(), None)]);
    }

    #[test]
    fn host_function_errors_are_raised_from_lua() {
        let host = ScriptHost::new().unwrap();
        let f = safe_function(host.lua(), |_, args| {
            if args.is_empty() {
                Err(mlua::Error::runtime("no arguments"))
            } else {
                Ok(args)
            }
        })
        .unwrap();
        host.lua().globals().set("echo", f).unwrap();
        let ok: (i64, String) = host
            .lua()
            .load("local ok, e = pcall(echo); assert(not ok); return echo(7, e)")
            .eval()
            .unwrap();
        assert_eq!(ok.0, 7);
        assert!(ok.1.contains("no arguments"), "{}", ok.1);
    }

    #[test]
    fn missing_globals_default_by_their_prefix() {
        let host = ScriptHost::new().unwrap();
        host.stub_missing_globals().unwrap();
        let (h, n, s, b): (f64, f64, String, bool) = host
            .lua()
            .load("tList[1] = 2; return nScreenHeight / 2, nCount + 1, sName .. 'x', bActive")
            .eval()
            .unwrap();
        assert_eq!((h, n, s.as_str(), b), (450.0, 1.0, "x", false));
        let no_object: bool = host.lua().load("return oFlash == nil").eval().unwrap();
        assert!(no_object);
        let is_stub: bool = host
            .lua()
            .load("return getmetatable(Event) ~= nil and tList[1] == 2")
            .eval()
            .unwrap();
        assert!(is_stub);
        let (sum, text): (f64, String) = host
            .lua()
            .load("local h = Graphics.GetHeight(); return h / 2 + 3, 'w=' .. h")
            .eval()
            .unwrap();
        assert_eq!((sum, text.as_str()), (3.0, "w=Graphics.GetHeight()"));
        // A script's own state reads nil until set, and after it is unset.
        let unset: bool = host
            .lua()
            .load("_bDone = true; _bDone = nil; return _bDone == nil and _tList == nil")
            .eval()
            .unwrap();
        assert!(unset);
    }

    #[test]
    fn library_members_not_written_answer_nil() {
        let host = ScriptHost::new().unwrap();
        host.lua()
            .load("Pg = { Known = function() return 1 end }")
            .exec()
            .unwrap();
        host.complete_libraries(&["Pg", "Nothing"]).unwrap();
        let (known, missing): (i64, Option<i64>) = host
            .lua()
            .load("return Pg.Known(), Pg.LoadSomething('x')")
            .eval()
            .unwrap();
        assert_eq!((known, missing), (1, None));
        assert_eq!(host.take_calls(), ["Pg.LoadSomething"]);
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
