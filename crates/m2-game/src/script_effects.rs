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

/// What these natives keep.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectsState {
    pub required_assets: Vec<RequiredAsset>,
    /// `Effect.Terrain` calls the game hasn't taken yet.
    pub terrain: Vec<TerrainEffect>,
    /// The three constants `CameraFade` last set.
    pub camera_fade: [f32; 3],
}

impl Default for EffectsState {
    fn default() -> Self {
        Self {
            required_assets: Vec::new(),
            terrain: Vec::new(),
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
