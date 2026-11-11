//! Missing natives the scripts call, second gap-list batch: system,
//! sound and player calls, ported from the 360 code with their
//! single-player answers (most used first). The 360 address of each is on
//! its function.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// How many achievement counters `Pg.AchievementAddCount` keeps (0x32).
pub const ACHIEVEMENT_COUNTERS: usize = 50;

/// What these natives keep for the game to read.
#[derive(Debug, Clone, PartialEq)]
pub struct SysState {
    /// `Net.QuitGame` asked to leave the game (message 5).
    pub quit: bool,
    /// `Player.SetInPmc`: the player's in-the-PMC flag (record +0x453).
    pub in_pmc: Option<bool>,
    /// `Pg.AchievementAddCount`'s counters (profile, from +0x348c).
    pub achievement_counts: [i32; ACHIEVEMENT_COUNTERS],
    /// Achievements `Pg.AchievementAddCount` granted, by name, in order.
    pub achievements: Vec<String>,
    /// `Human.StopGrappling`: characters told to stop grappling, in order.
    pub stop_grappling: Vec<u32>,
    /// `Sound.SetSurvivalMode`: the survival mix level, 1 on and 0 off.
    pub survival_mode: Option<f32>,
    /// `Sound.SetTimerUpdateMusic`: the music players' timer update flag.
    pub timer_update_music: Option<bool>,
    /// `Sound.StopAndFlushAllSounds` calls.
    pub flush_sounds: u32,
}

impl Default for SysState {
    fn default() -> Self {
        Self {
            quit: false,
            in_pmc: None,
            achievement_counts: [0; ACHIEVEMENT_COUNTERS],
            achievements: Vec::new(),
            stop_grappling: Vec::new(),
            survival_mode: None,
            timer_update_music: None,
            flush_sounds: 0,
        }
    }
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let sys = lib("Sys")?;
    add(&sys, "GetPlatform", |_, _, _| one(Value::Number(2.0)))?;
    add(&sys, "GetLanguage", get_language)?;
    add(&sys, "IsLoadingOrStreaming", is_loading_or_streaming)?;
    let net = lib("Net")?;
    add(&net, "QuitGame", quit_game)?;
    add(&net, "IsOnlineConnected", |_, _, _| {
        one(Value::Boolean(false))
    })?;
    add(&net, "HasPlayerUnlockedCode", |_, _, _| {
        one(Value::Boolean(false))
    })?;
    let player = lib("Player")?;
    add(&player, "SetInPmc", set_in_pmc)?;
    add(&player, "GetLocalPlayerId", get_local_player_id)?;
    add(&lib("Pg")?, "AchievementAddCount", achievement_add_count)?;
    add(&lib("Human")?, "StopGrappling", stop_grappling)?;
    let sound = lib("Sound")?;
    add(&sound, "SetSurvivalMode", set_survival_mode)?;
    add(&sound, "SetTimerUpdateMusic", set_timer_update_music)?;
    add(&sound, "StopAndFlushAllSounds", |w, _, _| {
        w.sys.flush_sounds += 1;
        nothing()
    })?;
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

fn integer(v: Option<&Value>) -> Option<i32> {
    match v? {
        Value::Integer(i) => i32::try_from(*i).ok(),
        Value::Number(n) if n.is_finite() => Some(*n as i32),
        _ => None,
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn nothing() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

fn string(lua: &Lua, s: &str) -> mlua::Result<MultiValue> {
    one(Value::String(lua.create_string(s)?))
}

/// Cuts `s` to at most `max` bytes, on a character boundary.
fn cut(mut s: String, max: usize) -> String {
    while s.len() > max {
        s.pop();
    }
    s
}

/// `Sys.GetLanguage()` (0x82345fc8): the language setting's name; the game
/// reads `English.wad`, so "English" (the 360's default case).
fn get_language(_: &mut ScriptWorld, lua: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    string(lua, "English")
}

/// `Sys.IsLoadingOrStreaming()` (0x823448d8): whether the game state is
/// `Loading` (hash 0x96fb0f27) or `WaitForStreaming` (0x7e289119).
fn is_loading_or_streaming(w: &mut ScriptWorld, _: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    let state = w.game_state.to_ascii_lowercase();
    one(Value::Boolean(
        state == "loading" || state == "waitforstreaming",
    ))
}

/// `Net.QuitGame()` (0x82328c40): asks to leave the game (message 5) and
/// returns true.
fn quit_game(w: &mut ScriptWorld, _: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    w.sys.quit = true;
    one(Value::Boolean(true))
}

/// `Player.SetInPmc(player, in)` (0x8233fe28): sets the flag and returns
/// true; nil without a guid and a boolean.
fn set_in_pmc(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match (guid(a.first()), boolean(a.get(1))) {
        (Some(_), Some(on)) => {
            w.sys.in_pmc = Some(on);
            one(Value::Boolean(true))
        }
        _ => one(Value::Nil),
    }
}

/// `Player.GetLocalPlayerId([n])` (0x8233e7c0): the local player's number,
/// -1 for a negative `n`. With one local player that is 0 (inferred: the
/// lookup 0x82467778 isn't traced further).
fn get_local_player_id(_: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let id = match integer(a.first()) {
        Some(n) if n < 0 => -1.0,
        _ => 0.0,
    };
    one(Value::Number(id))
}

/// `Pg.AchievementAddCount(name, counter, add, at)` (0x82338f58): adds
/// `add` (0 unless given) to counter `counter` (0 to 49) and grants the
/// achievement `name` as the count reaches `at` (0 unless given): when
/// `at <= count < at + add`. Needs a name and a counter of 0 or more;
/// always returns false.
fn achievement_add_count(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let name = text(a.first()).filter(|n| !n.is_empty());
    let counter = integer(a.get(1)).unwrap_or(-1);
    let add = integer(a.get(2)).unwrap_or(0);
    let at = integer(a.get(3)).unwrap_or(0);
    if let (Some(name), Ok(i)) = (name, usize::try_from(counter))
        && i < ACHIEVEMENT_COUNTERS
    {
        let count = w.sys.achievement_counts[i].wrapping_add(add);
        w.sys.achievement_counts[i] = count;
        if count < at.wrapping_add(add) && at <= count {
            w.sys.achievements.push(cut(name, 127));
        }
    }
    one(Value::Boolean(false))
}

/// `Human.StopGrappling(character)` (0x82325258): tells it to let go;
/// returns nothing, or nil without a guid.
fn stop_grappling(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match guid(a.first()) {
        Some(g) => {
            w.sys.stop_grappling.push(g);
            nothing()
        }
        None => one(Value::Nil),
    }
}

/// `Sound.SetSurvivalMode(on)` (0x82343538): the survival mix at 1 or 0;
/// returns nothing, or nil without a boolean.
fn set_survival_mode(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match boolean(a.first()) {
        Some(on) => {
            w.sys.survival_mode = Some(if on { 1.0 } else { 0.0 });
            nothing()
        }
        None => one(Value::Nil),
    }
}

/// `Sound.SetTimerUpdateMusic(on)` (0x823417b0): sets every music player's
/// timer update flag; returns nothing, or nil without a boolean.
fn set_timer_update_music(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match boolean(a.first()) {
        Some(on) => {
            w.sys.timer_update_music = Some(on);
            nothing()
        }
        None => one(Value::Nil),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Object, Shared, install};
    use crate::script::ScriptHost;

    fn setup() -> (ScriptHost, Shared) {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("Mattias", [0.0; 3]));
            w.player = Some(1);
        }
        install(host.lua(), &world).unwrap();
        (host, world)
    }

    #[test]
    fn system_sound_and_player_answers_from_the_360() {
        let (host, world) = setup();
        let lua = host.lua();
        let n = |code: &str| -> usize {
            lua.load(format!("return select('#', {code})"))
                .eval()
                .unwrap()
        };
        let got = |code: &str| -> Value { lua.load(format!("return {code}")).eval().unwrap() };
        let me = "Player.GetLocalPlayer()";
        // Fixed answers.
        assert_eq!(got("Sys.GetPlatform()"), Value::Number(2.0));
        let language: String = lua.load("return Sys.GetLanguage()").eval().unwrap();
        assert_eq!(language, "English");
        assert_eq!(got("Net.IsOnlineConnected()"), Value::Boolean(false));
        assert_eq!(got("Net.HasPlayerUnlockedCode()"), Value::Boolean(false));
        // Loading or streaming by the game state.
        assert_eq!(got("Sys.IsLoadingOrStreaming()"), Value::Boolean(false));
        world.borrow_mut().game_state = "WaitForStreaming".into();
        assert_eq!(got("Sys.IsLoadingOrStreaming()"), Value::Boolean(true));
        // Quit.
        assert_eq!(got("Net.QuitGame()"), Value::Boolean(true));
        assert!(world.borrow().sys.quit);
        // In the PMC.
        assert_eq!(
            got(&format!("Player.SetInPmc({me}, true)")),
            Value::Boolean(true)
        );
        assert_eq!(world.borrow().sys.in_pmc, Some(true));
        assert_eq!(got(&format!("Player.SetInPmc({me})")), Value::Nil);
        // Local player number.
        assert_eq!(got("Player.GetLocalPlayerId()"), Value::Number(0.0));
        assert_eq!(got("Player.GetLocalPlayerId(-1)"), Value::Number(-1.0));
        // Achievement counters: granted as the count reaches 3, once.
        for _ in 0..4 {
            assert_eq!(
                got("Pg.AchievementAddCount('ACH_KILLS', 7, 1, 3)"),
                Value::Boolean(false)
            );
        }
        assert_eq!(world.borrow().sys.achievement_counts[7], 4);
        assert_eq!(world.borrow().sys.achievements, ["ACH_KILLS"]);
        got("Pg.AchievementAddCount('ACH_X', 50, 1, 0)");
        assert_eq!(world.borrow().sys.achievements.len(), 1);
        // Grappling, survival mix, music timers, flush.
        assert_eq!(n(&format!("Human.StopGrappling({me})")), 0);
        assert_eq!(world.borrow().sys.stop_grappling, [1]);
        assert_eq!(got("Human.StopGrappling()"), Value::Nil);
        assert_eq!(n("Sound.SetSurvivalMode(true)"), 0);
        assert_eq!(world.borrow().sys.survival_mode, Some(1.0));
        assert_eq!(got("Sound.SetSurvivalMode()"), Value::Nil);
        assert_eq!(n("Sound.SetTimerUpdateMusic(false)"), 0);
        assert_eq!(world.borrow().sys.timer_update_music, Some(false));
        assert_eq!(n("Sound.StopAndFlushAllSounds()"), 0);
        assert_eq!(world.borrow().sys.flush_sounds, 1);
    }
}
