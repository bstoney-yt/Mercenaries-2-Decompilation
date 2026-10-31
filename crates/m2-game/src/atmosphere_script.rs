//! What an object script does to the light when it activates: the
//! satellite bomb's blast ("Explosion (Airstike Bomb)", its
//! `ObjectScript` 228f1fde) waits a tenth of a second (`Event.Create` of a
//! `TimerRelative` 0.1), then between `Graphics.Atmosphere.Begin` and
//! `End` turns the ambient light grey (128, 128, 128), the sunlight up to
//! 3.75, the air's haze off and the bloom up, with a `fTimeRestore` of
//! 1.7 s; see [`m2_engine::atmosphere::ScriptAtmosphere`] for what the
//! engine does with that (from the 360 code).
//!
//! The script is run as it is, in a VM of its own, with `Event.Create`
//! keeping its timers and `Graphics.Atmosphere` keeping its calls; its
//! timers are then fired in order of time. Other libraries it calls
//! (`MrxUtil.ShieldFace`) do nothing.

use m2_assets::lua::Chunk;
use m2_engine::atmosphere::{EditValue, ScriptAtmosphere};
use mlua::{Function, Table, Value};

use crate::script::{ScriptError, ScriptHost};

/// The libraries the script is run with, and the driver that activates it
/// and fires its timers (at most 16) in order of time.
const RUNNER: &str = r#"
__m2_timers, __m2_done, __m2_now = {}, {}, 0
local atmo
function import() end
MrxUtil = setmetatable({}, { __index = function() return function() end end })
Event = {
    TimerRelative = "TimerRelative",
    Create = function(kind, args, fn, data)
        local delay = type(args) == "table" and tonumber(args[1]) or 0
        table.insert(__m2_timers, { kind = kind, at = __m2_now + delay, fn = fn, data = data })
        return #__m2_timers
    end,
    Delete = function() end,
}
Graphics = { Atmosphere = {
    Begin = function() atmo = { at = __m2_now } end,
    SetValue = function(name, v)
        if atmo then table.insert(atmo, { name, v }) end
    end,
    SetColorValue = function(name, r, g, b, a)
        if atmo then table.insert(atmo, { name, r, g, b, a }) end
    end,
    End = function(t)
        if atmo then atmo.time = t; table.insert(__m2_done, atmo); atmo = nil end
    end,
} }
return function(activate)
    if type(activate) ~= "function" then return end
    activate(1)
    for _ = 1, 16 do
        local next
        for _, t in ipairs(__m2_timers) do
            if not t.fired and t.kind == "TimerRelative" and type(t.fn) == "function"
                and (not next or t.at < next.at) then
                next = t
            end
        end
        if not next then return end
        next.fired = true
        __m2_now = next.at
        if type(next.data) == "table" then next.fn(unpack(next.data)) else next.fn() end
    end
end
"#;

/// What `chunk` (an object script) does to the light when it activates:
/// how long after (seconds) and its first `Begin`..`End`, or `None` when
/// it doesn't touch the light.
pub fn activation_light(chunk: &Chunk) -> Result<Option<(f32, ScriptAtmosphere)>, ScriptError> {
    let host = ScriptHost::new()?;
    let lua = host.lua();
    let run: Function = lua.load(RUNNER).set_name("=m2-atmosphere").eval()?;
    host.load(chunk, "object script")?.call::<()>(())?;
    let activate: Value = lua.globals().get("OnActivate")?;
    run.call::<()>(activate)?;
    let done: Table = lua.globals().get("__m2_done")?;
    let Some(first) = done.sequence_values::<Table>().next() else {
        return Ok(None);
    };
    let first = first?;
    let mut edits = Vec::new();
    let mut restore = None;
    for call in first.clone().sequence_values::<Table>() {
        let call = call?;
        let name: String = call.get(1)?;
        let value = match call.raw_len() {
            2 => EditValue::Float(call.get(2)?),
            _ => {
                let n = |i: usize| -> mlua::Result<u32> { Ok(call.get::<f64>(i)? as u32) };
                EditValue::colour(n(2)?, n(3)?, n(4)?, n(5)?)
            }
        };
        match (name.as_str(), value) {
            ("fTimeRestore", EditValue::Float(r)) => restore = Some(r).filter(|r| *r >= 0.0),
            _ => edits.push((name, value)),
        }
    }
    let at: f32 = first.get("at")?;
    let interpolate: Option<f32> = first.get("time")?;
    Ok(Some((
        at,
        ScriptAtmosphere {
            edits,
            interpolate: interpolate.unwrap_or(0.0),
            restore,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compiles Lua source with the host's own compiler, as a stand-in for
    /// an object script from the archive.
    fn compile(src: &str) -> Chunk {
        let lua = mlua::Lua::new();
        let f = lua.load(src).into_function().unwrap();
        Chunk::parse(&f.dump(false)).unwrap()
    }

    /// The shape of the satellite bomb blast's script (228f1fde): a timer,
    /// then the light set in its callback.
    #[test]
    fn a_blast_s_light_is_read_with_its_delay_and_restore() {
        let chunk = compile(
            r#"
            import("MrxUtil")
            function OnActivate(obj)
                Event.Create(Event.TimerRelative, { 0.1 }, _GraphicsAto, { obj })
            end
            function _GraphicsAto(obj)
                Graphics.Atmosphere.Begin()
                Graphics.Atmosphere.SetColorValue("uiAmbientCube0", 128, 128, 128, 255)
                Graphics.Atmosphere.SetValue("fLightIntensity", 3.75)
                Graphics.Atmosphere.SetValue("fTimeRestore", 1.7)
                Graphics.Atmosphere.End()
                MrxUtil.ShieldFace(obj)
            end
            "#,
        );
        let (at, light) = activation_light(&chunk).unwrap().unwrap();
        assert!((at - 0.1).abs() < 1e-6);
        assert_eq!(
            light.edits,
            vec![
                ("uiAmbientCube0".to_owned(), EditValue::Colour(0xff80_8080)),
                ("fLightIntensity".to_owned(), EditValue::Float(3.75)),
            ]
        );
        assert_eq!(light.interpolate, 0.0);
        assert!((light.restore.unwrap() - 1.7).abs() < 1e-6);
    }

    #[test]
    fn a_script_that_leaves_the_light_alone_reads_as_none() {
        let chunk = compile("function OnActivate(obj) end");
        assert_eq!(activation_light(&chunk).unwrap(), None);
    }
}
