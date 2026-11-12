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

//! - `Graphics.Camera.SetFocusParams(view, n0, n1, f0, f1, amount [, t])`
//!   (`0x823185B8`) reads a view's number, five numbers and an optional
//!   time (0 without); nothing happens without the six. For a view under
//!   5 it makes the view's scripted focus the camera's default block
//!   (`0x821B6148`) with these five in place of its (0, 0.1, 100, 300,
//!   0.3), marks the focus scripted (+0x368) and starts a blend to it
//!   from where the focus is now, over `t` seconds (+0x360 elapsed,
//!   +0x364 length). The five are the depth of field's: sharp from `n1`
//!   (soft nearer than `n0`), soft from `f0` to `f1` and beyond, by
//!   `amount`. The hijack calls `(0, 0, 0.1, 4, 6, 1)`.
//! - `Graphics.Camera.RestoreFocusParams(view [, t])` (`0x82318758`)
//!   clears the mark and blends back to the camera's own focus over `t`
//!   seconds (0 without). Both answer nothing.
//!   The port keeps what was asked in [`EffectsState::focus`]; nothing
//!   draws from it yet.

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

/// The camera's own focus, where no script sets one (`0x821B6148`): near
/// start and end, far start and end, amount.
pub const FOCUS_DEFAULT: [f32; 5] = [0.0, 0.1, 100.0, 300.0, 0.3];

/// How many views the camera natives take (`view < 5`).
pub const VIEWS: usize = 5;

/// What the scripts last asked of a view's focus.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FocusAsk {
    /// `SetFocusParams`' five numbers; `None` after `RestoreFocusParams`
    /// (or before any call): the camera's own ([`FOCUS_DEFAULT`]).
    pub params: Option<[f32; 5]>,
    /// Seconds the change blends in over.
    pub seconds: f32,
    /// Counts the calls, so the game sees one that repeats the last.
    pub calls: u32,
}

/// What these natives keep.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectsState {
    pub required_assets: Vec<RequiredAsset>,
    /// The three constants `CameraFade` last set.
    pub camera_fade: [f32; 3],
    /// Each view's scripted focus (`Graphics.Camera.SetFocusParams`).
    pub focus: [FocusAsk; VIEWS],
}

impl Default for EffectsState {
    fn default() -> Self {
        Self {
            required_assets: Vec::new(),
            camera_fade: FADE_ON,
            focus: [FocusAsk::default(); VIEWS],
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

/// `Graphics.Camera.SetFocusParams`: see the module's notes.
fn set_focus_params(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(view) = int_arg(a, 1) else {
        return Ok(nil());
    };
    let mut params = [0.0; 5];
    for (k, p) in params.iter_mut().enumerate() {
        let Some(v) = num_arg(a, k + 2) else {
            return Ok(nil());
        };
        *p = v;
    }
    let seconds = num_arg(a, 7).unwrap_or(0.0);
    // The 360 compares the view as a signed number against 5 and takes a
    // negative one's address as it comes; none is given, and none is
    // taken here.
    if let Some(f) = usize::try_from(view)
        .ok()
        .and_then(|v| w.effects.focus.get_mut(v))
    {
        *f = FocusAsk {
            params: Some(params),
            seconds,
            calls: f.calls + 1,
        };
    }
    Ok(none())
}

/// `Graphics.Camera.RestoreFocusParams`: see the module's notes.
fn restore_focus_params(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(view) = int_arg(a, 1) else {
        return Ok(nil());
    };
    let seconds = num_arg(a, 2).unwrap_or(0.0);
    if let Some(f) = usize::try_from(view)
        .ok()
        .and_then(|v| w.effects.focus.get_mut(v))
    {
        *f = FocusAsk {
            params: None,
            seconds,
            calls: f.calls + 1,
        };
    }
    Ok(none())
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
    let camera = match graphics.raw_get::<Value>("Camera")? {
        Value::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            graphics.raw_set("Camera", &t)?;
            t
        }
    };
    add(&camera, "SetFocusParams", set_focus_params)?;
    add(&camera, "RestoreFocusParams", restore_focus_params)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_hash::engine_hash;

    #[test]
    fn the_focus_natives_keep_what_the_scripts_ask() {
        let lua = Lua::new();
        let mut w = ScriptWorld::default();
        let n = |v: f64| Value::Number(v);
        // The hijack's call: view 0, soft from 4 m to 6 m, fully, over 1 s.
        let hijack = [n(0.0), n(0.0), n(0.1), n(4.0), n(6.0), n(1.0), n(1.0)];
        assert_eq!(set_focus_params(&mut w, &lua, &hijack).unwrap().len(), 0);
        assert_eq!(
            w.effects.focus[0],
            FocusAsk {
                params: Some([0.0, 0.1, 4.0, 6.0, 1.0]),
                seconds: 1.0,
                calls: 1
            }
        );
        // Without the time, 0; a view of 5 or more is not taken.
        set_focus_params(
            &mut w,
            &lua,
            &[n(1.0), n(0.0), n(0.1), n(4.0), n(6.0), n(1.0)],
        )
        .unwrap();
        assert_eq!(w.effects.focus[1].seconds, 0.0);
        set_focus_params(
            &mut w,
            &lua,
            &[n(5.0), n(0.0), n(0.1), n(4.0), n(6.0), n(1.0)],
        )
        .unwrap();
        // Short of the five numbers: nil and nothing kept.
        assert_eq!(
            set_focus_params(&mut w, &lua, &hijack[..5]).unwrap().len(),
            1
        );
        assert_eq!(w.effects.focus[0].calls, 1);
        // Restored: the camera's own again, blending over the time given.
        restore_focus_params(&mut w, &lua, &[n(0.0), n(0.5)]).unwrap();
        assert_eq!(
            w.effects.focus[0],
            FocusAsk {
                params: None,
                seconds: 0.5,
                calls: 2
            }
        );
        restore_focus_params(&mut w, &lua, &[n(1.0)]).unwrap();
        assert_eq!(w.effects.focus[1].seconds, 0.0);
        assert_eq!(restore_focus_params(&mut w, &lua, &[]).unwrap().len(), 1);
    }

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
