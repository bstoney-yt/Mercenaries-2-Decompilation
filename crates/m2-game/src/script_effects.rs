//! `Sys.RequiredAsset` and the `Graphics.Effect` natives: `CameraFade`,
//! and `Terrain`, `AmbientTop` and `AmbientSides` until the port has
//! scripted effects.
//!
//! From the 360 code:
//! - `Sys.RequiredAsset(name, kind [, n] [, flag])` (`0x82344D70`) reads
//!   two strings (hashed), an optional integer and an optional boolean,
//!   adds them to the level's required-asset list (`0x82219018`) and
//!   answers the result, true here; nil without both strings. The level
//!   bootstrap names its `_base` layer and its scripts with it.
//! - `Graphics.Effect.CameraFade(on)` (`0x8231A488`) reads an integer and
//!   sets three render constants at `0x82B80B48` (`0x821E41C8`): (2.25,
//!   1.25, 1.0) for nonzero, the values the image starts with, and (0.1,
//!   0.0, 0.1) for zero. It answers nothing; nil without a number. The
//!   hijack sets 0 on entry and 1 on exit. The three are far, near and
//!   range of the particles' fade close to the camera (the particle sort
//!   pass, `0x821E3440`), which the alpha's particle draw applies from
//!   [`EffectsState::camera_fade`] (`m2_engine::particles::camera_fade`).
//! - `Graphics.Effect.Terrain(name, a [, b] [, at])` (`0x8231A5D0`) and
//!   `AmbientTop`/`AmbientSides(name, slot, a [, b])` (`0x8231A4F0`,
//!   `0x8231A6D0`) spawn an effect object from the named model (kinds 6 and
//!   2, `0x821DA650`/`0x821DA518`) and answer its guid, or nil.
//!
//! Stand-in: the port has no scripted effect objects, so the three
//! spawning calls check their arguments and answer nil, which the 360
//! answers when the effect can't be made. The real answer is the effect's
//! guid; scripts that keep it (the dust falls, the bunker buster's ground
//! shockwave) get nil for now.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;
use crate::script_sound::{bool_arg, hash_arg, nil, none, num_arg, yes_no};

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// The render constants `CameraFade` sets (`0x82B80B48`).
pub const FADE_ON: [f32; 3] = [2.25, 1.25, 1.0];
pub const FADE_OFF: [f32; 3] = [0.1, 0.0, 0.1];

/// One `Sys.RequiredAsset` entry: name and kind hashes, number, flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequiredAsset {
    pub name: u32,
    pub kind: u32,
    pub n: i32,
    pub flag: bool,
}

/// What these natives keep.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectsState {
    pub required_assets: Vec<RequiredAsset>,
    /// The three constants `CameraFade` last set.
    pub camera_fade: [f32; 3],
}

impl Default for EffectsState {
    fn default() -> Self {
        Self {
            required_assets: Vec::new(),
            camera_fade: FADE_ON,
        }
    }
}

/// Argument `i` (from 1) as the engine's integer reader (`0x82308320`)
/// takes it: a number, truncated.
fn int_arg(args: &[Value], i: usize) -> Option<i32> {
    num_arg(args, i).map(|n| n as i32)
}

fn required_asset(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(name), Some(kind)) = (hash_arg(a, 1), hash_arg(a, 2)) else {
        return Ok(nil());
    };
    // The optional integer, then the optional boolean after it.
    let (n, next) = match int_arg(a, 3) {
        Some(n) => (n, 4),
        None => (0, 3),
    };
    let flag = bool_arg(a, next).unwrap_or(false);
    w.effects.required_assets.push(RequiredAsset {
        name,
        kind,
        n,
        flag,
    });
    Ok(yes_no(true))
}

fn camera_fade(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(on) = int_arg(a, 1) else {
        return Ok(nil());
    };
    w.effects.camera_fade = if on != 0 { FADE_ON } else { FADE_OFF };
    Ok(none())
}

/// `Effect.Terrain` / `AmbientTop` / `AmbientSides`: nil whatever the
/// arguments (the 360's answer without an effect; stand-in, see above).
fn no_effect(_: &mut ScriptWorld, _: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    Ok(nil())
}

pub(crate) fn install(
    lua: &Lua,
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    add(&lib("Sys")?, "RequiredAsset", required_asset)?;
    let graphics = lib("Graphics")?;
    let effect = match graphics.raw_get::<Value>("Effect")? {
        Value::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            graphics.raw_set("Effect", &t)?;
            t
        }
    };
    let handlers: [(&str, Handler); 4] = [
        ("CameraFade", camera_fade),
        ("Terrain", no_effect),
        ("AmbientTop", no_effect),
        ("AmbientSides", no_effect),
    ];
    for (name, f) in handlers {
        add(&effect, name, f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_hash::engine_hash;

    fn s(lua: &Lua, t: &str) -> Value {
        Value::String(lua.create_string(t).unwrap())
    }

    #[test]
    fn required_assets_are_listed() {
        let lua = Lua::new();
        let mut w = ScriptWorld::default();
        let r = required_asset(
            &mut w,
            &lua,
            &[s(&lua, "_base"), s(&lua, "layer"), Value::Integer(-2)],
        )
        .unwrap();
        assert_eq!(r.into_iter().collect::<Vec<_>>(), [Value::Boolean(true)]);
        assert_eq!(
            w.effects.required_assets,
            [RequiredAsset {
                name: engine_hash(b"_base"),
                kind: engine_hash(b"layer"),
                n: -2,
                flag: false
            }]
        );
        let r = required_asset(&mut w, &lua, &[s(&lua, "_base")]).unwrap();
        assert_eq!(r.into_iter().collect::<Vec<_>>(), [Value::Nil]);
        assert_eq!(w.effects.required_assets.len(), 1);
    }

    #[test]
    fn camera_fade_switches_the_constants() {
        let lua = Lua::new();
        let mut w = ScriptWorld::default();
        assert_eq!(w.effects.camera_fade, FADE_ON);
        assert!(
            camera_fade(&mut w, &lua, &[Value::Integer(0)])
                .unwrap()
                .is_empty()
        );
        assert_eq!(w.effects.camera_fade, FADE_OFF);
        assert!(
            camera_fade(&mut w, &lua, &[Value::Number(1.0)])
                .unwrap()
                .is_empty()
        );
        assert_eq!(w.effects.camera_fade, FADE_ON);
        assert_eq!(camera_fade(&mut w, &lua, &[]).unwrap().len(), 1);
        assert_eq!(w.effects.camera_fade, FADE_ON);
    }

    #[test]
    fn spawning_effects_answer_nil_until_the_port_has_them() {
        let lua = Lua::new();
        let mut w = ScriptWorld::default();
        let dust = [
            s(&lua, "global_particle_dustfall"),
            Value::Integer(20),
            Value::Number(0.005),
        ];
        assert_eq!(
            no_effect(&mut w, &lua, &dust)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            [Value::Nil]
        );
        assert_eq!(no_effect(&mut w, &lua, &[]).unwrap().len(), 1);
    }
}
