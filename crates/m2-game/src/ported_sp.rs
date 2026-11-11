//! Missing natives the scripts call, ported from the 360 code as a single
//! player game answers them (gap list, most used first).
//!
//! From the 360 code, per function below. Several are network calls whose
//! single-player answer is fixed: the 360 tests one flag (0x82de698c,
//! read by every `Net` call that talks to other machines) before doing
//! anything, and with it clear they return their offline value. That the
//! flag means "a network session is running" is inferred from which calls
//! test it.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// What these natives keep for the game to read.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SpState {
    /// `Player.SetSeatMovementLocks`: the player's three seat movement
    /// locks (player record +0x44d, +0x44e, +0x44f), each true unless
    /// given.
    pub seat_movement_locks: Option<[bool; 3]>,
    /// `Gui.ShowLoadingHints(true)` asked for the loading hints.
    pub loading_hints: bool,
    /// `Net.SendEvent_RecruitsUnlocked`: the unlocked recruits as the 360
    /// packs them: Fiona 1, Copter 2, Mechanic 4, Pilot 8.
    pub recruits_unlocked: Option<u32>,
    /// `Net.GrantAchievement`: the achievements granted, by name, in order.
    pub achievements: Vec<String>,
    /// `Animation.PlayFacialExpression`: the expressions asked for.
    pub facial: Vec<FacialExpression>,
    /// `Animation.SetUseBriefingLOD`: the briefing detail switch (global
    /// 0x83134483).
    pub briefing_lod: bool,
}

/// One `Animation.PlayFacialExpression(character, expression, a, b, flag)`.
/// What `a` (1 unless given), `b` (-1 unless given) and `flag` (true unless
/// given) mean is not traced past the queue they go to (0x822552e8).
#[derive(Debug, Clone, PartialEq)]
pub struct FacialExpression {
    pub character: u32,
    pub expression: String,
    pub a: f32,
    pub b: f32,
    pub flag: bool,
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let player = lib("Player")?;
    add(&player, "SetSeatMovementLocks", set_seat_movement_locks)?;
    add(&lib("Gui")?, "ShowLoadingHints", show_loading_hints)?;
    let net = lib("Net")?;
    add(&net, "IsMatchmakingInternet", offline_false)?;
    add(&net, "IsOnlineEnabled", offline_false)?;
    add(&net, "SendEvent_RecruitsUnlocked", recruits_unlocked)?;
    add(&net, "GrantAchievement", grant_achievement)?;
    add(&net, "SendEvent_PursuitMessage", offline_true)?;
    add(&net, "SetPursuitReportingState", offline_true)?;
    let animation = lib("Animation")?;
    add(&animation, "PlayFacialExpression", play_facial_expression)?;
    add(&animation, "SetUseBriefingLOD", set_use_briefing_lod)?;
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

/// `Player.SetSeatMovementLocks(player, a, b, c)` (0x8233dad8): sets the
/// player's three locks (each true unless given a boolean) and returns
/// true; nil without a guid. Any guid reads the local player's record, as
/// the 360's lookup falls back to it.
fn set_seat_movement_locks(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if guid(a.first()).is_none() {
        return one(Value::Nil);
    }
    let lock = |i: usize| boolean(a.get(i)).unwrap_or(true);
    w.sp.seat_movement_locks = Some([lock(1), lock(2), lock(3)]);
    one(Value::Boolean(true))
}

/// `Gui.ShowLoadingHints([on])` (0x8231c678): on (true unless given false)
/// shows the loading hints; false does nothing. Returns nothing.
fn show_loading_hints(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if boolean(a.first()).unwrap_or(true) {
        w.sp.loading_hints = true;
    }
    nothing()
}

/// `Net.IsMatchmakingInternet()` and `Net.IsOnlineEnabled()` (one 360
/// function, 0x8232d068): true only in a network session on the internet;
/// false offline.
fn offline_false(_: &mut ScriptWorld, _: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    one(Value::Boolean(false))
}

/// `Net.SendEvent_PursuitMessage` (0x8232bfc8) and
/// `Net.SetPursuitReportingState` (0x8232c0e8): with no network session
/// they skip their arguments and return true.
fn offline_true(_: &mut ScriptWorld, _: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    one(Value::Boolean(true))
}

/// `Net.SendEvent_RecruitsUnlocked({ Fiona = b, Copter = b, Mechanic = b,
/// Pilot = b })` (0x8232dc80): packs the true ones as bits 1, 2, 4 and 8
/// and hands them to the game (0x82468fb0, message 3); returns true. With
/// no table every bit is clear.
fn recruits_unlocked(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let mut mask = 0;
    if let Some(Value::Table(t)) = a.first() {
        for (bit, name) in [(1, "Fiona"), (2, "Copter"), (4, "Mechanic"), (8, "Pilot")] {
            if matches!(t.raw_get::<Value>(name)?, Value::Boolean(true)) {
                mask |= bit;
            }
        }
    }
    w.sp.recruits_unlocked = Some(mask);
    one(Value::Boolean(true))
}

/// `Net.GrantAchievement(name)` (0x8232b158): queues the achievement
/// (message 0x29, the name cut to 127 bytes) and returns nothing.
fn grant_achievement(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if let Some(name) = text(a.first()).filter(|n| !n.is_empty()) {
        let mut cut = name;
        while cut.len() > 127 {
            cut.pop();
        }
        w.sp.achievements.push(cut);
    }
    nothing()
}

/// `Animation.PlayFacialExpression(character, expression, [a], [b],
/// [flag])` (0x82316078): queues the expression; returns nothing, or nil
/// without a character and an expression.
fn play_facial_expression(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(character), Some(expression)) = (guid(a.first()), text(a.get(1))) else {
        return one(Value::Nil);
    };
    w.sp.facial.push(FacialExpression {
        character,
        expression,
        a: number(a.get(2)).unwrap_or(1.0),
        b: number(a.get(3)).unwrap_or(-1.0),
        flag: boolean(a.get(4)).unwrap_or(true),
    });
    nothing()
}

/// `Animation.SetUseBriefingLOD(on)` (0x823162b0): sets the switch;
/// returns nothing, or nil without a boolean.
fn set_use_briefing_lod(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match boolean(a.first()) {
        Some(on) => {
            w.sp.briefing_lod = on;
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
    fn single_player_answers_from_the_360() {
        let (host, world) = setup();
        let lua = host.lua();
        let n = |code: &str| -> usize {
            lua.load(format!("return select('#', {code})"))
                .eval()
                .unwrap()
        };
        let got = |code: &str| -> Value { lua.load(format!("return {code}")).eval().unwrap() };
        let me = "Player.GetLocalPlayer()";
        // Seat locks: true unless given; nil without a guid.
        assert_eq!(
            got(&format!("Player.SetSeatMovementLocks({me}, false)")),
            Value::Boolean(true)
        );
        assert_eq!(
            world.borrow().sp.seat_movement_locks,
            Some([false, true, true])
        );
        assert_eq!(got("Player.SetSeatMovementLocks(nil)"), Value::Nil);
        // Loading hints: false does nothing; nothing returned.
        assert_eq!(n("Gui.ShowLoadingHints(false)"), 0);
        assert!(!world.borrow().sp.loading_hints);
        assert_eq!(n("Gui.ShowLoadingHints()"), 0);
        assert!(world.borrow().sp.loading_hints);
        // Offline network answers.
        assert_eq!(got("Net.IsMatchmakingInternet()"), Value::Boolean(false));
        assert_eq!(got("Net.IsOnlineEnabled()"), Value::Boolean(false));
        assert_eq!(
            got("Net.SendEvent_PursuitMessage(1, 'x')"),
            Value::Boolean(true)
        );
        assert_eq!(got("Net.SetPursuitReportingState()"), Value::Boolean(true));
        // Recruits as bits.
        assert_eq!(
            got("Net.SendEvent_RecruitsUnlocked({ Fiona = true, Mechanic = true, Pilot = false })"),
            Value::Boolean(true)
        );
        assert_eq!(world.borrow().sp.recruits_unlocked, Some(5));
        // Achievements: kept by name; nothing returned.
        assert_eq!(n("Net.GrantAchievement('ACH_TEST')"), 0);
        assert_eq!(world.borrow().sp.achievements, ["ACH_TEST"]);
        // Facial expressions: defaults 1, -1, true; nil without a face.
        assert_eq!(
            n(&format!("Animation.PlayFacialExpression({me}, 'smile')")),
            0
        );
        assert_eq!(
            world.borrow().sp.facial,
            [FacialExpression {
                character: 1,
                expression: "smile".into(),
                a: 1.0,
                b: -1.0,
                flag: true
            }]
        );
        assert_eq!(
            got("Animation.PlayFacialExpression(nil, 'smile')"),
            Value::Nil
        );
        // The briefing detail switch.
        assert_eq!(n("Animation.SetUseBriefingLOD(true)"), 0);
        assert!(world.borrow().sp.briefing_lod);
        assert_eq!(got("Animation.SetUseBriefingLOD()"), Value::Nil);
    }
}
