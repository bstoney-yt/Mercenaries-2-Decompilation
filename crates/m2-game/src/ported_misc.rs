//! Missing natives the scripts call, fourth gap-list batch: setters and
//! simple questions ported from the 360 code. Each function names its 360
//! address.

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// How many subjects the AI keeps (`Ai.AddSubject`'s table at +0x3ce4).
pub const AI_SUBJECTS: usize = 8;

/// What these natives keep for the game to read.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MiscState {
    /// `Sys.SetTimeScale`: the game's time scale (clamped as the 360 does).
    pub time_scale: Option<f32>,
    /// `Sys.SetLuaSaveVersion`: the scripts' save version (0x83134470).
    pub lua_save_version: Option<i32>,
    /// `Sys.SetNumberOfViewports`: 1 to 4.
    pub viewports: Option<i32>,
    /// `Sys.SetINIBriefing` (0x83134065).
    pub ini_briefing: Option<bool>,
    /// `Human.SetJostleEnabled`, by character.
    pub jostle: HashMap<u32, bool>,
    /// `Object.StopAllAnimation`: objects told to stop, in order.
    pub stop_all_animation: Vec<u32>,
    /// `Pg.SetBoundaryRadius` (+0x1ed4; its square at +0x1ed8).
    pub boundary_radius: Option<f32>,
    /// `Graphics.SetGamma`: red, green and blue.
    pub gamma: Option<[f32; 3]>,
    /// `Sound.SetHijackMusic`: two hashes (+0xd91c, +0xd920).
    pub hijack_music: Option<[u32; 2]>,
    /// `Sound.SetSourceMusic`: a hash (+0xd3d8).
    pub source_music: Option<u32>,
    /// `Sound.RequestAmbienceBank`: banks asked for, in order.
    pub ambience_banks: Vec<u32>,
    /// `Ai.AddSubject`'s slots: 0 is free.
    pub ai_subjects: [u32; AI_SUBJECTS],
    /// `Junk.ToggleAlarm`: objects whose alarm was toggled, in order.
    pub alarms: Vec<u32>,
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let sys = lib("Sys")?;
    add(&sys, "SetTimeScale", set_time_scale)?;
    add(&sys, "SetLuaSaveVersion", set_lua_save_version)?;
    add(&sys, "SetNumberOfViewports", set_number_of_viewports)?;
    add(&sys, "SetINIBriefing", set_ini_briefing)?;
    add(&lib("Human")?, "SetJostleEnabled", set_jostle_enabled)?;
    let object = lib("Object")?;
    add(&object, "StopAllAnimation", stop_all_animation)?;
    add(&object, "IsTemplate", is_template)?;
    add(&object, "OutsideBoundary", outside_boundary)?;
    let pg = lib("Pg")?;
    add(&pg, "SetBoundaryRadius", set_boundary_radius)?;
    add(&pg, "GetBoundaryRadius", get_boundary_radius)?;
    add(&lib("Graphics")?, "SetGamma", set_gamma)?;
    let sound = lib("Sound")?;
    add(&sound, "SetHijackMusic", set_hijack_music)?;
    add(&sound, "SetSourceMusic", set_source_music)?;
    add(&sound, "RequestAmbienceBank", request_ambience_bank)?;
    let ai = lib("Ai")?;
    add(&ai, "AddSubject", add_subject)?;
    add(&ai, "RemoveSubject", remove_subject)?;
    add(&lib("Junk")?, "ToggleAlarm", toggle_alarm)?;
    Ok(())
}

fn guid(v: Option<&Value>) -> Option<u32> {
    crate::events::as_guid(v?)
}

fn boolean(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

fn integer(v: Option<&Value>) -> Option<i32> {
    match v? {
        // Through f64: mlua::Integer is i32 on i686 and i64 elsewhere.
        Value::Integer(i) => Some(*i as f64 as i32),
        Value::Number(n) if n.is_finite() => Some(*n as i32),
        _ => None,
    }
}

/// A hash argument: a name (hashed as the archive's names are) or the
/// number itself.
fn hash(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) => Some(m2_assets::name_hash(&s.to_string_lossy())),
        Value::Integer(i) => Some(*i as u32),
        Value::Number(n) if n.is_finite() => Some(*n as i64 as u32),
        _ => None,
    }
}

fn nothing() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

fn nil() -> mlua::Result<MultiValue> {
    one(Value::Nil)
}

/// `Sys.SetTimeScale(scale)` (0x82344a40): below 0.0001 is 0, above 1 is
/// 1, and a scale whose 0.005th is at most 0.0001 (0.02 or less) is 0;
/// returns true, or nil without a number.
fn set_time_scale(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(mut s) = number(a.first()) else {
        return nil();
    };
    if s < 1.0e-4 {
        s = 0.0;
    }
    s = s.clamp(0.0, 1.0);
    if s * 0.005 <= 1.0e-4 {
        s = 0.0;
    }
    w.misc.time_scale = Some(s);
    one(Value::Boolean(true))
}

/// `Sys.SetLuaSaveVersion(n)` (0x82345c80): returns nothing, or nil
/// without a number.
fn set_lua_save_version(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match integer(a.first()) {
        Some(n) => {
            w.misc.lua_save_version = Some(n);
            nothing()
        }
        None => nil(),
    }
}

/// `Sys.SetNumberOfViewports(n)` (0x823449a0): 1 to 4 is kept and returns
/// true; anything else is nil.
fn set_number_of_viewports(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match integer(a.first()).filter(|n| (1..5).contains(n)) {
        Some(n) => {
            w.misc.viewports = Some(n);
            one(Value::Boolean(true))
        }
        None => nil(),
    }
}

/// `Sys.SetINIBriefing(on)` (0x823450d8): keeps the switch and returns
/// nil, as it does without a boolean.
fn set_ini_briefing(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if let Some(on) = boolean(a.first()) {
        w.misc.ini_briefing = Some(on);
    }
    nil()
}

/// `Human.SetJostleEnabled(character, on)` (0x82325a50): sets the flag on
/// its components; returns nothing, or nil without both arguments.
fn set_jostle_enabled(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match (guid(a.first()), boolean(a.get(1))) {
        (Some(g), Some(on)) => {
            w.misc.jostle.insert(g, on);
            nothing()
        }
        _ => nil(),
    }
}

/// `Object.StopAllAnimation(object)` (0x823326d8): returns nothing, or nil
/// without a guid.
fn stop_all_animation(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match guid(a.first()) {
        Some(g) => {
            w.misc.stop_all_animation.push(g);
            nothing()
        }
        None => nil(),
    }
}

/// `Object.IsTemplate(guid)` (0x8232e4d8): whether the guid's top bit is
/// set (template guids); nil without a guid.
fn is_template(_: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match guid(a.first()) {
        Some(g) => one(Value::Boolean(g & 0x8000_0000 != 0)),
        None => nil(),
    }
}

/// `Object.OutsideBoundary(object, boundary)` (0x823312b0): whether the
/// object is outside the boundary's region (as `InsideBoundary`, negated);
/// nil when either has no place or isn't a guid.
fn outside_boundary(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let inside = match (guid(a.first()), guid(a.get(1))) {
        (Some(o), Some(b)) => crate::events::World::inside(&*w, o, b),
        _ => None,
    };
    match inside {
        Some(inside) => one(Value::Boolean(!inside)),
        None => nil(),
    }
}

/// `Pg.SetBoundaryRadius(r)` (0x82338408): returns nothing, or nil without
/// a number.
fn set_boundary_radius(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match number(a.first()) {
        Some(r) => {
            w.misc.boundary_radius = Some(r);
            nothing()
        }
        None => nil(),
    }
}

/// `Pg.GetBoundaryRadius()` (0x82338478): the radius set; 0 before one is
/// set (invented: the engine's starting value isn't traced).
fn get_boundary_radius(w: &mut ScriptWorld, _: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    one(Value::Number(f64::from(
        w.misc.boundary_radius.unwrap_or(0.0),
    )))
}

/// `Graphics.SetGamma(r, g, b)` (0x82317f78): returns nothing, or nil
/// without three numbers.
fn set_gamma(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match (number(a.first()), number(a.get(1)), number(a.get(2))) {
        (Some(r), Some(g), Some(b)) => {
            w.misc.gamma = Some([r, g, b]);
            nothing()
        }
        _ => nil(),
    }
}

/// `Sound.SetHijackMusic(a, b)` (0x82342080): two hashes; returns nothing,
/// or nil without both.
fn set_hijack_music(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match (hash(a.first()), hash(a.get(1))) {
        (Some(x), Some(y)) => {
            w.misc.hijack_music = Some([x, y]);
            nothing()
        }
        _ => nil(),
    }
}

/// `Sound.SetSourceMusic(hash)` (0x82341e08): returns nothing, or nil
/// without a hash.
fn set_source_music(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match hash(a.first()) {
        Some(h) => {
            w.misc.source_music = Some(h);
            nothing()
        }
        None => nil(),
    }
}

/// `Sound.RequestAmbienceBank(hash)` (0x82342d88): asks for the bank (0
/// asks for none); returns nothing, or nil without a hash.
fn request_ambience_bank(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match hash(a.first()) {
        Some(h) => {
            if h != 0 {
                w.misc.ambience_banks.push(h);
            }
            nothing()
        }
        None => nil(),
    }
}

/// `Ai.AddSubject(object)` (0x82312f58, 0x822f6788): puts it in the first
/// free of 8 slots and returns true; false when they are full or without a
/// guid. It doesn't look for the object already there.
fn add_subject(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let added = guid(a.first()).filter(|&g| g != 0).is_some_and(|g| {
        let free = w.misc.ai_subjects.iter_mut().find(|s| **s == 0);
        free.map(|s| *s = g).is_some()
    });
    one(Value::Boolean(added))
}

/// `Ai.RemoveSubject(object)` (0x82312fe8): clears the first slot holding
/// it and returns true; false when none does or without a guid.
fn remove_subject(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let removed = guid(a.first()).filter(|&g| g != 0).is_some_and(|g| {
        let slot = w.misc.ai_subjects.iter_mut().find(|s| **s == g);
        slot.map(|s| *s = 0).is_some()
    });
    one(Value::Boolean(removed))
}

/// `Junk.ToggleAlarm(object)` (0x82327380): toggles its alarm (message 2);
/// returns nothing, or nil without a guid.
fn toggle_alarm(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match guid(a.first()) {
        Some(g) => {
            w.misc.alarms.push(g);
            nothing()
        }
        None => nil(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Object, Shared, install};
    use crate::script::ScriptHost;

    #[test]
    fn setters_and_questions_answer_as_the_360() {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("Mattias", [10.0, 0.0, 5.0]));
            w.player = Some(1);
            w.insert(70, Object::new("rgn", [0.0, 0.0, 0.0]));
            w.regions.insert(
                70,
                m2_assets::Region {
                    flags: 0x0004_0002,
                    unk_0x04: 0,
                    corners: vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]],
                },
            );
        }
        install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let n = |code: &str| -> usize {
            lua.load(format!("return select('#', {code})"))
                .eval()
                .unwrap()
        };
        let got = |code: &str| -> Value { lua.load(format!("return {code}")).eval().unwrap() };
        let me = "Player.GetLocalPlayer()";
        let w = || world.borrow().misc.clone();
        // Time scale: clamped.
        for (s, want) in [(0.5, 0.5), (2.0, 1.0), (0.01, 0.0), (-1.0, 0.0)] {
            assert_eq!(got(&format!("Sys.SetTimeScale({s})")), Value::Boolean(true));
            assert_eq!(w().time_scale, Some(want), "{s}");
        }
        assert_eq!(got("Sys.SetTimeScale()"), Value::Nil);
        // Simple setters.
        assert_eq!(n("Sys.SetLuaSaveVersion(3)"), 0);
        assert_eq!(w().lua_save_version, Some(3));
        assert_eq!(got("Sys.SetNumberOfViewports(2)"), Value::Boolean(true));
        assert_eq!(got("Sys.SetNumberOfViewports(5)"), Value::Nil);
        assert_eq!(w().viewports, Some(2));
        assert_eq!(n("Sys.SetINIBriefing(true)"), 1);
        assert_eq!(w().ini_briefing, Some(true));
        assert_eq!(n(&format!("Human.SetJostleEnabled({me}, false)")), 0);
        assert_eq!(w().jostle.get(&1), Some(&false));
        assert_eq!(n(&format!("Object.StopAllAnimation({me})")), 0);
        assert_eq!(w().stop_all_animation, [1]);
        // Templates by the guid's top bit.
        assert_eq!(
            got(&format!("Object.IsTemplate({me})")),
            Value::Boolean(false)
        );
        assert_eq!(got("Object.IsTemplate(nil)"), Value::Nil);
        // Outside a region: the hero at (10, 5) is outside the 10 m square.
        assert_eq!(
            got(&format!(
                "Object.OutsideBoundary({me}, Pg.GetGuidByName('rgn'))"
            )),
            Value::Boolean(true)
        );
        // Boundary radius.
        assert_eq!(got("Pg.GetBoundaryRadius()"), Value::Number(0.0));
        assert_eq!(n("Pg.SetBoundaryRadius(250)"), 0);
        assert_eq!(got("Pg.GetBoundaryRadius()"), Value::Number(250.0));
        // Gamma and sound.
        assert_eq!(n("Graphics.SetGamma(1, 0.9, 0.8)"), 0);
        assert_eq!(got("Graphics.SetGamma(1)"), Value::Nil);
        assert_eq!(n("Sound.SetHijackMusic('a', 'b')"), 0);
        assert_eq!(
            w().hijack_music,
            Some([m2_assets::name_hash("a"), m2_assets::name_hash("b")])
        );
        assert_eq!(n("Sound.SetSourceMusic('m')"), 0);
        assert_eq!(n("Sound.RequestAmbienceBank('amb')"), 0);
        assert_eq!(w().ambience_banks, [m2_assets::name_hash("amb")]);
        // AI subjects: 8 slots, no duplicate check.
        for _ in 0..8 {
            assert_eq!(got(&format!("Ai.AddSubject({me})")), Value::Boolean(true));
        }
        assert_eq!(got(&format!("Ai.AddSubject({me})")), Value::Boolean(false));
        assert_eq!(
            got(&format!("Ai.RemoveSubject({me})")),
            Value::Boolean(true)
        );
        assert_eq!(w().ai_subjects.iter().filter(|&&s| s == 1).count(), 7);
        assert_eq!(got("Ai.RemoveSubject(nil)"), Value::Boolean(false));
        // Alarms.
        assert_eq!(n(&format!("Junk.ToggleAlarm({me})")), 0);
        assert_eq!(w().alarms, [1]);
    }
}
