//! Small `Sys`, `Gui`, `Player`, `Object` and `Graphics` natives the
//! shell, the level bootstrap and the post-processing set-up call:
//! the level and master script names, the version, tutorials, bloom and
//! contrast.
//!
//! From the 360 code:
//! - `Sys.SetLevelName` / `SetMasterScriptName` (`0x82344C18` /
//!   `0x82344C88`) copy the name into a 64-byte buffer (`0x83133F18` /
//!   `0x83133FD8`, through `0x82308390`) and answer true, or nil if it
//!   came out empty. `GetMasterScriptName` (`0x82344BA0`) answers that
//!   buffer (nil while empty). The boot fills both from the configuration
//!   (`0x822193D8`: the level name, copied into the master script name),
//!   so the port answers the level name for an unset master script name
//!   and "vz" for an unset level (inferred: the configuration's value isn't
//!   read). `GetCharacterTemplate` (`0x82344CF8`) does
//!   the same with `0x8313406C`, which nothing the scripts call sets.
//! - `Sys.GetVersion` (`0x82346268`) answers two numbers, each clamped to
//!   100000..999999; both words (`0x831340C0`/`C4`) are 0 in the image, so
//!   100000 twice unless something sets them (inferred: no writer found).
//! - `Sys.SetTutorialsEnabled(on)` (`0x82345BB8`) writes the profile's
//!   tutorial option ("On"/"Off"); nil without a boolean.
//! - `Gui.DoSigninCheck` / `OnGlobalExit` (`0x8231C570` / `0x8231C648`)
//!   call the shell and answer nothing. `Gui.IsPdaOnSelect` (`0x8231AB50`)
//!   answers the byte at `0x82B767CD`, 1 in the image.
//! - `Player.GetAvailableCostumes` (`0x8233FCC8`) answers the profile byte
//!   at options +0x222.
//! - `Object.RemoveQualityRef` (`0x82333170`) releases the object's model
//!   quality reference and always answers nil, even on success.
//! - `Graphics.AA.SetThreshold` (`0x8231A310`) reads a number and stores
//!   nothing. `Graphics.Bloom.SetBlurRadius`/`SetThreshold`/`SetMultiplier`
//!   and `Graphics.Contrast.SetLimit`/`SetMultiplier` store their number at
//!   `0x836508E8`/`EC`/`F0` and `0x83650904`/`08`. All answer nil without a
//!   number and nothing otherwise.
//!
//! Invented: `GetAvailableCostumes` answers 0 (a fresh profile's byte, not
//! traced). The renderer doesn't read the bloom and contrast values yet.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;
use crate::script_sound::{bool_arg, nil, none, num_arg, number, yes_no};

// Helper: extract text from argument (1-based)
pub fn text_arg(args: &[Value], i: usize) -> Option<String> {
    let val = args.get(i.checked_sub(1)?)?;
    match val {
        Value::String(s) => Some(s.to_string_lossy()),
        Value::Integer(n) => Some(n.to_string()),
        Value::Number(n) => Some(format!("{n}")),
        _ => None,
    }
}

// Shell state for the script shell
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShellState {
    pub level_name: Option<String>,
    pub master_script_name: Option<String>,
    pub bloom_blur_radius: Option<f32>,
    pub bloom_threshold: Option<f32>,
    pub bloom_multiplier: Option<f32>,
    pub contrast_limit: Option<f32>,
    pub contrast_multiplier: Option<f32>,
}

// Handler type alias
type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

// --- Native handlers ---

// Sys.SetLevelName(name)
/// A name argument as the engine copies it into its 64-byte buffer
/// (`0x82308390`): at most 63 bytes, and none if empty.
fn name_arg(args: &[Value]) -> Option<String> {
    let mut name = text_arg(args, 1)?;
    let mut end = name.len().min(63);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name.truncate(end);
    (!name.is_empty()).then_some(name)
}

fn sys_set_level_name(w: &mut ScriptWorld, _lua: &Lua, args: &[Value]) -> mlua::Result<MultiValue> {
    let Some(name) = name_arg(args) else {
        return Ok(nil());
    };
    w.shell.level_name = Some(name);
    Ok(yes_no(true))
}

// Sys.SetMasterScriptName(name)
fn sys_set_master_script_name(
    w: &mut ScriptWorld,
    _lua: &Lua,
    args: &[Value],
) -> mlua::Result<MultiValue> {
    let Some(name) = name_arg(args) else {
        return Ok(nil());
    };
    w.shell.master_script_name = Some(name);
    Ok(yes_no(true))
}

// Sys.GetMasterScriptName()
fn sys_get_master_script_name(
    w: &mut ScriptWorld,
    lua: &Lua,
    _args: &[Value],
) -> mlua::Result<MultiValue> {
    // The boot fills the buffer from the configuration's level name
    // (`0x822193D8` copies the level into the master script name), so the
    // shell finds a string here (its bootstrap ASSERTs one and passes it
    // to `Sys.StartSingleplayer`): the level name until one is set.
    let name = w
        .shell
        .master_script_name
        .clone()
        .or_else(|| w.shell.level_name.clone())
        .unwrap_or_else(|| "vz".to_owned());
    Ok(MultiValue::from_vec(vec![Value::String(
        lua.create_string(&name)?,
    )]))
}

// Sys.GetLevelName()
fn sys_get_level_name(w: &mut ScriptWorld, lua: &Lua, _args: &[Value]) -> mlua::Result<MultiValue> {
    let name = w.shell.level_name.as_deref().unwrap_or("vz");
    Ok(MultiValue::from_vec(vec![Value::String(
        lua.create_string(name)?,
    )]))
}

// Sys.GetCharacterTemplate()
fn sys_get_character_template(
    _w: &mut ScriptWorld,
    _lua: &Lua,
    _args: &[Value],
) -> mlua::Result<MultiValue> {
    Ok(nil())
}

// Sys.GetVersion()
fn sys_get_version(_w: &mut ScriptWorld, _lua: &Lua, _args: &[Value]) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![
        Value::Number(100000.0),
        Value::Number(100000.0),
    ]))
}

// Sys.SetTutorialsEnabled(on)
fn sys_set_tutorials_enabled(
    w: &mut ScriptWorld,
    _lua: &Lua,
    args: &[Value],
) -> mlua::Result<MultiValue> {
    let on = match bool_arg(args, 1) {
        Some(b) => b,
        None => return Ok(nil()),
    };
    w.tutorials_off = !on;
    Ok(none())
}

// Gui.DoSigninCheck()
fn gui_do_signin_check(
    _w: &mut ScriptWorld,
    _lua: &Lua,
    _args: &[Value],
) -> mlua::Result<MultiValue> {
    Ok(none())
}

// Gui.OnGlobalExit()
fn gui_on_global_exit(
    _w: &mut ScriptWorld,
    _lua: &Lua,
    _args: &[Value],
) -> mlua::Result<MultiValue> {
    Ok(none())
}

// Gui.IsPdaOnSelect()
fn gui_is_pda_on_select(
    _w: &mut ScriptWorld,
    _lua: &Lua,
    _args: &[Value],
) -> mlua::Result<MultiValue> {
    Ok(yes_no(true))
}

// Player.GetAvailableCostumes()
fn player_get_available_costumes(
    _w: &mut ScriptWorld,
    _lua: &Lua,
    _args: &[Value],
) -> mlua::Result<MultiValue> {
    Ok(number(0.0))
}

// Object.RemoveQualityRef(obj)
fn object_remove_quality_ref(
    _w: &mut ScriptWorld,
    _lua: &Lua,
    _args: &[Value],
) -> mlua::Result<MultiValue> {
    Ok(nil())
}

// Graphics.AA.SetThreshold(x)
fn graphics_aa_set_threshold(
    _w: &mut ScriptWorld,
    _lua: &Lua,
    args: &[Value],
) -> mlua::Result<MultiValue> {
    if num_arg(args, 1).is_some() {
        // Read and dropped: 0x8231A310 stores nothing.
        Ok(none())
    } else {
        Ok(nil())
    }
}

// Graphics.Bloom.SetBlurRadius(x)
fn graphics_bloom_set_blur_radius(
    w: &mut ScriptWorld,
    _lua: &Lua,
    args: &[Value],
) -> mlua::Result<MultiValue> {
    if let Some(x) = num_arg(args, 1) {
        w.shell.bloom_blur_radius = Some(x);
        Ok(none())
    } else {
        Ok(nil())
    }
}

// Graphics.Bloom.SetThreshold(x)
fn graphics_bloom_set_threshold(
    w: &mut ScriptWorld,
    _lua: &Lua,
    args: &[Value],
) -> mlua::Result<MultiValue> {
    if let Some(x) = num_arg(args, 1) {
        w.shell.bloom_threshold = Some(x);
        Ok(none())
    } else {
        Ok(nil())
    }
}

// Graphics.Bloom.SetMultiplier(x)
fn graphics_bloom_set_multiplier(
    w: &mut ScriptWorld,
    _lua: &Lua,
    args: &[Value],
) -> mlua::Result<MultiValue> {
    if let Some(x) = num_arg(args, 1) {
        w.shell.bloom_multiplier = Some(x);
        Ok(none())
    } else {
        Ok(nil())
    }
}

// Graphics.Contrast.SetLimit(x)
fn graphics_contrast_set_limit(
    w: &mut ScriptWorld,
    _lua: &Lua,
    args: &[Value],
) -> mlua::Result<MultiValue> {
    if let Some(x) = num_arg(args, 1) {
        w.shell.contrast_limit = Some(x);
        Ok(none())
    } else {
        Ok(nil())
    }
}

// Graphics.Contrast.SetMultiplier(x)
fn graphics_contrast_set_multiplier(
    w: &mut ScriptWorld,
    _lua: &Lua,
    args: &[Value],
) -> mlua::Result<MultiValue> {
    if let Some(x) = num_arg(args, 1) {
        w.shell.contrast_multiplier = Some(x);
        Ok(none())
    } else {
        Ok(nil())
    }
}

// --- Installation ---

pub(crate) fn install(
    lua: &Lua,
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    // Sys library
    let sys = lib("Sys")?;
    let sys_handlers: [(&str, Handler); 7] = [
        ("SetLevelName", sys_set_level_name),
        ("SetMasterScriptName", sys_set_master_script_name),
        ("GetMasterScriptName", sys_get_master_script_name),
        ("GetLevelName", sys_get_level_name),
        ("GetCharacterTemplate", sys_get_character_template),
        ("GetVersion", sys_get_version),
        ("SetTutorialsEnabled", sys_set_tutorials_enabled),
    ];
    for (name, f) in sys_handlers {
        add(&sys, name, f)?;
    }

    // Gui library
    let gui = lib("Gui")?;
    let gui_handlers: [(&str, Handler); 3] = [
        ("DoSigninCheck", gui_do_signin_check),
        ("OnGlobalExit", gui_on_global_exit),
        ("IsPdaOnSelect", gui_is_pda_on_select),
    ];
    for (name, f) in gui_handlers {
        add(&gui, name, f)?;
    }

    // Player library
    let player = lib("Player")?;
    let player_handlers: [(&str, Handler); 1] =
        [("GetAvailableCostumes", player_get_available_costumes)];
    for (name, f) in player_handlers {
        add(&player, name, f)?;
    }

    // Object library
    let object = lib("Object")?;
    let object_handlers: [(&str, Handler); 1] = [("RemoveQualityRef", object_remove_quality_ref)];
    for (name, f) in object_handlers {
        add(&object, name, f)?;
    }

    // Graphics.AA sublibrary
    let aa = {
        let t = lib("Graphics")?;
        match t.raw_get::<Value>("AA")? {
            Value::Table(tt) => tt,
            _ => {
                let new = lua.create_table()?;
                t.raw_set("AA", &new)?;
                new
            }
        }
    };
    let aa_handlers: [(&str, Handler); 1] = [("SetThreshold", graphics_aa_set_threshold)];
    for (name, f) in aa_handlers {
        add(&aa, name, f)?;
    }

    // Graphics.Bloom sublibrary
    let bloom = {
        let t = lib("Graphics")?;
        match t.raw_get::<Value>("Bloom")? {
            Value::Table(tt) => tt,
            _ => {
                let new = lua.create_table()?;
                t.raw_set("Bloom", &new)?;
                new
            }
        }
    };
    let bloom_handlers: [(&str, Handler); 3] = [
        ("SetBlurRadius", graphics_bloom_set_blur_radius),
        ("SetThreshold", graphics_bloom_set_threshold),
        ("SetMultiplier", graphics_bloom_set_multiplier),
    ];
    for (name, f) in bloom_handlers {
        add(&bloom, name, f)?;
    }

    // Graphics.Contrast sublibrary
    let contrast = {
        let t = lib("Graphics")?;
        match t.raw_get::<Value>("Contrast")? {
            Value::Table(tt) => tt,
            _ => {
                let new = lua.create_table()?;
                t.raw_set("Contrast", &new)?;
                new
            }
        }
    };
    let contrast_handlers: [(&str, Handler); 2] = [
        ("SetLimit", graphics_contrast_set_limit),
        ("SetMultiplier", graphics_contrast_set_multiplier),
    ];
    for (name, f) in contrast_handlers {
        add(&contrast, name, f)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(lua: &Lua, t: &str) -> Value {
        Value::String(lua.create_string(t).unwrap())
    }

    fn texts(m: MultiValue) -> Vec<String> {
        m.into_iter()
            .map(|v| match v {
                Value::String(s) => s.to_string_lossy(),
                Value::Nil => "nil".into(),
                v => format!("{v:?}"),
            })
            .collect()
    }

    #[test]
    fn names_are_kept_and_answered() {
        let lua = Lua::new();
        let mut w = ScriptWorld::default();
        assert_eq!(
            texts(sys_get_level_name(&mut w, &lua, &[]).unwrap()),
            ["vz"]
        );
        assert_eq!(
            texts(sys_get_master_script_name(&mut w, &lua, &[]).unwrap()),
            ["vz"]
        );
        assert_eq!(
            texts(sys_set_level_name(&mut w, &lua, &[s(&lua, "")]).unwrap()),
            ["nil"]
        );
        sys_set_level_name(&mut w, &lua, &[s(&lua, "vz_island")]).unwrap();
        assert_eq!(
            texts(sys_get_level_name(&mut w, &lua, &[]).unwrap()),
            ["vz_island"]
        );
        let long = "x".repeat(70);
        sys_set_master_script_name(&mut w, &lua, &[s(&lua, &long)]).unwrap();
        assert_eq!(
            w.shell.master_script_name.as_deref().map(str::len),
            Some(63)
        );
        assert_eq!(
            texts(sys_get_character_template(&mut w, &lua, &[]).unwrap()),
            ["nil"]
        );
        assert_eq!(sys_get_version(&mut w, &lua, &[]).unwrap().len(), 2);
    }

    #[test]
    fn setters_need_their_argument() {
        let lua = Lua::new();
        let mut w = ScriptWorld::default();
        assert!(
            sys_set_tutorials_enabled(&mut w, &lua, &[Value::Boolean(false)])
                .unwrap()
                .is_empty()
        );
        assert!(w.tutorials_off);
        assert_eq!(
            sys_set_tutorials_enabled(&mut w, &lua, &[Value::Integer(0)])
                .unwrap()
                .len(),
            1
        );
        assert!(
            graphics_bloom_set_threshold(&mut w, &lua, &[Value::Number(0.75)])
                .unwrap()
                .is_empty()
        );
        assert_eq!(w.shell.bloom_threshold, Some(0.75));
        assert_eq!(
            texts(graphics_contrast_set_limit(&mut w, &lua, &[]).unwrap()),
            ["nil"]
        );
        assert_eq!(w.shell.contrast_limit, None);
        assert_eq!(
            texts(object_remove_quality_ref(&mut w, &lua, &[Value::Integer(5)]).unwrap()),
            ["nil"]
        );
    }
}
