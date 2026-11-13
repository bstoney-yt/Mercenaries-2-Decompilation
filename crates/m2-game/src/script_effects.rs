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
//! - `Graphics.Effect.Terrain(name, a [, b] [, x, y, z])` (`0x8231A5D0`)
//!   reads the template's name (hashed), a number, an optional number
//!   (1 without) and an optional point (a global's without), and spawns
//!   the template's effect over the ground there (kind 6, `0x821DA650`):
//!   `a` is the patch's half-size, `b`, not below 0, the particles per
//!   unit of ground a frame. Nil without the name and `a`. The scripts
//!   (the dust falls: `("global_particle_dustfall", 20, 0.005, x, 8, z)`;
//!   the bunker buster's ground shockwave) drop the answer.
//! - `AmbientTop`/`AmbientSides(name, slot, a [, b])` (`0x8231A4F0`,
//!   `0x8231A6D0`) spawn an effect kept about the camera (kind 2,
//!   `0x821DA518`) and answer its guid, or nil. No script calls them.
//!
//! Stand-ins: `Terrain` hands the game a [`TerrainEffect`] and answers
//! nil, where the 360 answers the effect object's guid (the port has no
//! scripted effect objects; no script keeps the answer). Without a point
//! the 360 takes a global's (`0x82E806A0`, not traced): the port makes no
//! effect then. `AmbientTop` and `AmbientSides` answer nil.

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
//! - `Graphics.Camera.SetFovParams(view, fov [, t])` (`0x823183C0`) and
//!   `SetNearFar(view, near, far [, t])` (`0x823181C0`) do the same for
//!   the view's two other parameter sets (view +0x370 and +0x4D0, with
//!   their own elapsed time, length and scripted flag): the default block
//!   with the field of view (given in degrees, kept times 0.01745 in
//!   radians; the camera's own is 0.95974994) or the near and far
//!   distances (the camera's own 0.2 and 2400) in place. `RestoreFovParams`
//!   (`0x823184E8`) and `RestoreNearFar` (`0x823182F0`) `(view [, t])`
//!   clear the flag and blend back. All answer nothing; nil without the
//!   view and the values. The scripts call `SetNearFar(0, 0.3, 500, 0)`.
//!   Kept in [`EffectsState::fov`] and [`EffectsState::near_far`]; nothing
//!   draws from them yet.

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

/// One `Graphics.Effect.Terrain` call, for the game to carry out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainEffect {
    /// The template's name hash.
    pub template: u32,
    /// The ground patch's half-size, metres.
    pub radius: f32,
    /// Particles per unit of ground (by the game's count) a frame.
    pub density: f32,
    pub at: [f32; 3],
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

/// The camera's own field of view, radians, and near and far distances
/// (`0x821B6148`).
pub const FOV_DEFAULT: f32 = 0.959_749_94;
pub const NEAR_FAR_DEFAULT: [f32; 2] = [0.2, 2400.0];

/// Degrees to radians as `SetFovParams` converts them (`0x820F8C74`).
pub const FOV_DEGREES: f32 = 0.01745;

/// What the scripts last asked of one of a view's blended camera sets:
/// the values `Set...` gave (`None` after `Restore...` or before any
/// call: the camera's own), the seconds the change blends in over, and a
/// count of the calls, so the game sees one that repeats the last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraAsk<T> {
    pub params: Option<T>,
    pub seconds: f32,
    pub calls: u32,
}

impl<T> Default for CameraAsk<T> {
    fn default() -> Self {
        Self {
            params: None,
            seconds: 0.0,
            calls: 0,
        }
    }
}

/// What these natives keep.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectsState {
    pub required_assets: Vec<RequiredAsset>,
    /// `Effect.Terrain` calls the game hasn't taken yet.
    pub terrain: Vec<TerrainEffect>,
    /// The three constants `CameraFade` last set.
    pub camera_fade: [f32; 3],
    /// Each view's scripted focus (`Graphics.Camera.SetFocusParams`).
    pub focus: [FocusAsk; VIEWS],
    /// Each view's scripted field of view, radians (`SetFovParams`).
    pub fov: [CameraAsk<f32>; VIEWS],
    /// Each view's scripted near and far distances (`SetNearFar`).
    pub near_far: [CameraAsk<[f32; 2]>; VIEWS],
}

impl Default for EffectsState {
    fn default() -> Self {
        Self {
            required_assets: Vec::new(),
            terrain: Vec::new(),
            camera_fade: FADE_ON,
            focus: [FocusAsk::default(); VIEWS],
            fov: [CameraAsk::default(); VIEWS],
            near_far: [CameraAsk::default(); VIEWS],
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

/// `Graphics.Effect.Terrain(name, a [, b] [, x, y, z])`: see the module's
/// notes. As the 360's readers step through the arguments: `b` is the
/// third when that is a number, and the point the three after it.
fn terrain(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(template), Some(radius)) = (hash_arg(a, 1), num_arg(a, 2)) else {
        return Ok(nil());
    };
    let (density, first) = match num_arg(a, 3) {
        Some(b) => (b, 4),
        None => (1.0, 3),
    };
    let point = (
        num_arg(a, first),
        num_arg(a, first + 1),
        num_arg(a, first + 2),
    );
    if let (Some(x), Some(y), Some(z)) = point {
        w.effects.terrain.push(TerrainEffect {
            template,
            radius,
            density: density.max(0.0),
            at: [x, y, z],
        });
    }
    Ok(nil())
}

/// `AmbientTop` / `AmbientSides`: nil whatever the
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

/// Records a `Set...` (`Some`) or `Restore...` (`None`) for `view`, as
/// the 360 takes views under 5 (a negative one is not taken: see
/// `set_focus_params`).
fn ask<T: Copy>(slots: &mut [CameraAsk<T>; VIEWS], view: i32, params: Option<T>, seconds: f32) {
    if let Some(a) = usize::try_from(view).ok().and_then(|v| slots.get_mut(v)) {
        *a = CameraAsk {
            params,
            seconds,
            calls: a.calls + 1,
        };
    }
}

/// `Graphics.Camera.SetFovParams(view, fov [, t])`: see the module's notes.
fn set_fov_params(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(view), Some(degrees)) = (int_arg(a, 1), num_arg(a, 2)) else {
        return Ok(nil());
    };
    let seconds = num_arg(a, 3).unwrap_or(0.0);
    ask(
        &mut w.effects.fov,
        view,
        Some(degrees * FOV_DEGREES),
        seconds,
    );
    Ok(none())
}

/// `Graphics.Camera.SetNearFar(view, near, far [, t])`.
fn set_near_far(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(view), Some(near), Some(far)) = (int_arg(a, 1), num_arg(a, 2), num_arg(a, 3)) else {
        return Ok(nil());
    };
    let seconds = num_arg(a, 4).unwrap_or(0.0);
    ask(&mut w.effects.near_far, view, Some([near, far]), seconds);
    Ok(none())
}

/// `Graphics.Camera.RestoreFovParams(view [, t])`.
fn restore_fov_params(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(view) = int_arg(a, 1) else {
        return Ok(nil());
    };
    ask(&mut w.effects.fov, view, None, num_arg(a, 2).unwrap_or(0.0));
    Ok(none())
}

/// `Graphics.Camera.RestoreNearFar(view [, t])`.
fn restore_near_far(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(view) = int_arg(a, 1) else {
        return Ok(nil());
    };
    ask(
        &mut w.effects.near_far,
        view,
        None,
        num_arg(a, 2).unwrap_or(0.0),
    );
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
        ("Terrain", terrain),
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
    add(&camera, "SetFovParams", set_fov_params)?;
    add(&camera, "RestoreFovParams", restore_fov_params)?;
    add(&camera, "SetNearFar", set_near_far)?;
    add(&camera, "RestoreNearFar", restore_near_far)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_hands_the_game_the_dust_falls_call() {
        let lua = Lua::new();
        let mut w = ScriptWorld::default();
        let name = Value::String(lua.create_string("global_particle_dustfall").unwrap());
        // The dust fall's own call: name, 20, 0.005, x, 8, z.
        let args = [
            name.clone(),
            Value::Integer(20),
            Value::Number(0.005),
            Value::Number(100.0),
            Value::Integer(8),
            Value::Number(-40.0),
        ];
        assert_eq!(terrain(&mut w, &lua, &args).unwrap().len(), 1);
        let t = w.effects.terrain[0];
        assert_eq!(t.template, hash_arg(&args, 1).unwrap());
        assert_eq!((t.radius, t.density), (20.0, 0.005));
        assert_eq!(t.at, [100.0, 8.0, -40.0]);
        // A density below 0 is 0. Without a point: nothing asked (the
        // 360 takes a global's place, not traced).
        let low = [
            name.clone(),
            Value::Integer(5),
            Value::Number(-1.0),
            Value::Number(1.0),
            Value::Number(2.0),
            Value::Number(3.0),
        ];
        terrain(&mut w, &lua, &low).unwrap();
        assert_eq!(w.effects.terrain[1].density, 0.0);
        assert_eq!(w.effects.terrain[1].at, [1.0, 2.0, 3.0]);
        terrain(&mut w, &lua, &[name.clone(), Value::Integer(5)]).unwrap();
        // No name or no size: nil and nothing asked.
        terrain(&mut w, &lua, &[name]).unwrap();
        terrain(&mut w, &lua, &[]).unwrap();
        assert_eq!(w.effects.terrain.len(), 2);
    }
    use crate::script_hash::engine_hash;

    #[test]
    fn the_fov_and_near_far_natives_keep_what_the_scripts_ask() {
        let lua = Lua::new();
        let mut w = ScriptWorld::default();
        let n = |v: f64| Value::Number(v);
        // The scripts' call: view 0, near 0.3, far 500, at once.
        let r = set_near_far(&mut w, &lua, &[n(0.0), n(0.3), n(500.0), n(0.0)]).unwrap();
        assert_eq!(r.len(), 0);
        assert_eq!(
            w.effects.near_far[0],
            CameraAsk {
                params: Some([0.3, 500.0]),
                seconds: 0.0,
                calls: 1
            }
        );
        // Degrees in, radians kept, times the game's 0.01745.
        set_fov_params(&mut w, &lua, &[n(0.0), n(40.0), n(1.5)]).unwrap();
        assert_eq!(
            w.effects.fov[0],
            CameraAsk {
                params: Some(40.0 * 0.01745),
                seconds: 1.5,
                calls: 1
            }
        );
        // Restored: the camera's own again, over the time given (0 without).
        restore_fov_params(&mut w, &lua, &[n(0.0), n(1.0)]).unwrap();
        assert_eq!(
            (w.effects.fov[0].params, w.effects.fov[0].seconds),
            (None, 1.0)
        );
        assert_eq!(w.effects.fov[0].calls, 2);
        restore_near_far(&mut w, &lua, &[n(0.0)]).unwrap();
        assert_eq!(w.effects.near_far[0].params, None);
        assert_eq!(w.effects.near_far[0].calls, 2);
        // Short of their values: nil, nothing kept. A view of 5: not taken.
        assert_eq!(
            set_near_far(&mut w, &lua, &[n(0.0), n(0.3)]).unwrap().len(),
            1
        );
        assert_eq!(set_fov_params(&mut w, &lua, &[n(0.0)]).unwrap().len(), 1);
        assert_eq!(restore_fov_params(&mut w, &lua, &[]).unwrap().len(), 1);
        set_fov_params(&mut w, &lua, &[n(5.0), n(40.0)]).unwrap();
        assert_eq!(w.effects.near_far[0].calls, 2);
        assert_eq!(w.effects.fov[0].calls, 2);
    }

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
