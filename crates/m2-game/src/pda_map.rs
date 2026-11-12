//! The player's PDA map mode: the engine state behind the satellite view
//! a support designator opens (`MrxSupportDesignatorSatellite`), as the
//! scripts drive it through `Player`.
//!
//! From the 360 code (the natives are in the `Player` table; the state is
//! an object the player keeps, made and freed by `0x8243A8D8`, run by
//! `0x82441DD8`):
//!
//! - `Player.SetPDAMapMode(player, on, [x, y, z], [radius], [zoom in],
//!   [zoom out], [minigame])` (`0x8233C550`): on, with no mode up, makes
//!   the state with what was given: where the view's camera starts, how
//!   far from there it may go, how far it may come down and go up from
//!   its start (0 and 250 when left out) and whether the timing minigame
//!   confirms; off, ends it. True for a player and a flag, else nil. With
//!   a mode up, on changes nothing.
//! - `Player.SetPDAMapModeCallback(player, which, callback)`
//!   (`0x8233C138`, in [`crate::ported`]): the mode's two callbacks. The
//!   confirm handler (`0x82442268`) calls the `false` one with the hold's
//!   progress while confirm is held and the `true` one when the mode is
//!   confirmed or asked to end, each with the player, the point under the
//!   view (its height the ground's there plus 1) and the progress, the
//!   `true` one also with what is under the view.
//! - `Player.SetPDAMapModeCancelCallback(player, callback)`
//!   (`0x8233C240`): called when the mode is cancelled.
//! - `Player.RequestPDAMapModeExit(player, callback)` (`0x8233C310`):
//!   asks the mode to end at its next update, with `callback` in place
//!   of the `true` callback.
//! - `Player.RequestPDAMapModeCancel(player)` (`0x8233C3E8`): asks it to
//!   cancel at its next update, as the cancel button does.
//!
//! The three callback natives answer true for good arguments whether a
//! mode is up or not, and keep nothing when none is.
//!
//! From the scripts: the satellite designator turns the mode on at
//! (x, y + `nStartZoom`, z) over the player's character with `nRadius`,
//! `nStartZoom - nMinZoom`, `nMaxZoom - nStartZoom` and
//! `MrxGuiSatellite.UseMinigame()`, and off from its `true` callback.
//!
//! This module keeps the state for the game to run; the view itself
//! (the camera, the buttons, the hold) is the game's.

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;
use crate::events::as_guid;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// How far the view may go up from its start when the scripts don't say
/// (the 360's default, 250.0).
pub const ZOOM_OUT: f32 = 250.0;

/// The PDA map mode while it is up.
#[derive(Debug, Clone, Default)]
pub struct MapMode {
    /// Whose it is.
    pub player: u32,
    /// Where the view's camera starts, if the scripts said.
    pub centre: Option<[f32; 3]>,
    /// How far from there it may go, if they said.
    pub radius: Option<f32>,
    /// How far it may come down and go up from its start.
    pub zoom_in: f32,
    pub zoom_out: f32,
    /// The timing minigame confirms, not the hold.
    pub minigame: bool,
    /// Called when the mode is cancelled.
    pub cancel: Option<Function>,
    /// Asked to end at its next update, with this in place of the
    /// `true` callback.
    pub exit: Option<Function>,
    /// Asked to cancel at its next update.
    pub cancel_asked: bool,
}

fn values<I: IntoIterator<Item = Value>>(v: I) -> mlua::Result<MultiValue> {
    Ok(v.into_iter().collect())
}

fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

fn flag(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

fn set_mode(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(player), Some(on)) = (a.first().and_then(as_guid), flag(a.get(1))) else {
        return values([Value::Nil]);
    };
    if !on {
        // Ending frees the state and its callbacks.
        w.pda_map = None;
        w.pda_map_callbacks = [None, None];
        return values([Value::Boolean(true)]);
    }
    if w.pda_map.is_some() {
        return values([Value::Boolean(true)]);
    }
    // The numbers after the flag, in order: a place takes three.
    let numbers: Vec<f32> = a[2..]
        .iter()
        .map_while(|v| number(Some(v)))
        .take(6)
        .collect();
    let (centre, rest) = match numbers.as_slice() {
        [x, y, z, rest @ ..] => (Some([*x, *y, *z]), rest),
        rest => (None, rest),
    };
    let minigame = flag(a.get(2 + numbers.len())).unwrap_or(false);
    w.pda_map = Some(MapMode {
        player,
        centre,
        radius: rest.first().copied(),
        zoom_in: rest.get(1).copied().unwrap_or(0.0),
        zoom_out: rest.get(2).copied().unwrap_or(ZOOM_OUT),
        minigame,
        cancel: None,
        exit: None,
        cancel_asked: false,
    });
    values([Value::Boolean(true)])
}

fn set_cancel_callback(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(_), Some(Value::Function(f))) = (a.first().and_then(as_guid), a.get(1)) else {
        return values([Value::Nil]);
    };
    if let Some(m) = &mut w.pda_map {
        m.cancel = Some(f.clone());
    }
    values([Value::Boolean(true)])
}

fn request_exit(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(_), Some(Value::Function(f))) = (a.first().and_then(as_guid), a.get(1)) else {
        return values([Value::Nil]);
    };
    if let Some(m) = &mut w.pda_map {
        m.exit = Some(f.clone());
    }
    values([Value::Boolean(true)])
}

fn request_cancel(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if a.first().and_then(as_guid).is_none() {
        return values([Value::Nil]);
    }
    if let Some(m) = &mut w.pda_map {
        m.cancel_asked = true;
    }
    values([Value::Boolean(true)])
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let player = lib("Player")?;
    add(&player, "SetPDAMapMode", set_mode)?;
    add(&player, "SetPDAMapModeCancelCallback", set_cancel_callback)?;
    add(&player, "RequestPDAMapModeExit", request_exit)?;
    add(&player, "RequestPDAMapModeCancel", request_cancel)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::api::{self, ScriptWorld, Shared};
    use crate::script::ScriptHost;

    fn world() -> (ScriptHost, Shared) {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        (host, world)
    }

    /// The designator's call, as its script makes it: the place, the
    /// radius, the two zoom rooms and the minigame flag.
    #[test]
    fn the_satellite_designators_call_makes_the_mode() {
        let (host, world) = world();
        let ok: bool = host
            .lua()
            .load(
                r#"
                me = Player.GetLocalPlayer()
                local nStartZoom, nMinZoom, nMaxZoom = 170, 120, 250
                return Player.SetPDAMapMode(me, true, 10, 5 + nStartZoom, -20, 100,
                    nStartZoom - nMinZoom, nMaxZoom - nStartZoom, true)
                "#,
            )
            .eval()
            .unwrap();
        assert!(ok);
        let w = world.borrow();
        let m = w.pda_map.as_ref().unwrap();
        assert_eq!(m.centre, Some([10.0, 175.0, -20.0]));
        assert_eq!(m.radius, Some(100.0));
        assert_eq!((m.zoom_in, m.zoom_out, m.minigame), (50.0, 80.0, true));
        assert!(m.cancel.is_none() && m.exit.is_none() && !m.cancel_asked);
    }

    /// What is left out takes the 360's defaults; a mode already up is
    /// left as it is; off ends it and frees the callbacks.
    #[test]
    fn defaults_and_ending() {
        let (host, world) = world();
        host.lua()
            .load(
                r#"
                me = Player.GetLocalPlayer()
                assert(Player.SetPDAMapMode(me, true) == true)
                assert(Player.SetPDAMapMode(me, true, 1, 2, 3, 9) == true)
                assert(Player.SetPDAMapModeCallback(me, true, function() end) == true)
                assert(Player.SetPDAMapModeCancelCallback(me, function() end) == true)
                "#,
            )
            .exec()
            .unwrap();
        {
            let w = world.borrow();
            let m = w.pda_map.as_ref().unwrap();
            assert_eq!((m.centre, m.radius), (None, None));
            assert_eq!(
                (m.zoom_in, m.zoom_out, m.minigame),
                (0.0, super::ZOOM_OUT, false)
            );
            assert!(m.cancel.is_some() && w.pda_map_callbacks[1].is_some());
        }
        host.lua()
            .load("assert(Player.SetPDAMapMode(me, false) == true)")
            .exec()
            .unwrap();
        let w = world.borrow();
        assert!(w.pda_map.is_none());
        assert!(w.pda_map_callbacks.iter().all(Option::is_none));
    }

    /// The requests mark a mode that is up, answer true without one, and
    /// nil for bad arguments.
    #[test]
    fn requests_mark_the_mode() {
        let (host, world) = world();
        host.lua()
            .load(
                r#"
                me = Player.GetLocalPlayer()
                -- No mode: true all the same, nothing kept.
                assert(Player.RequestPDAMapModeCancel(me) == true)
                assert(Player.RequestPDAMapModeExit(me, function() end) == true)
                assert(Player.SetPDAMapModeCancelCallback(me, function() end) == true)
                -- Bad arguments: nil.
                assert(Player.SetPDAMapMode(me) == nil)
                assert(Player.SetPDAMapMode("me", true) == nil)
                assert(Player.SetPDAMapModeCancelCallback(me, 5) == nil)
                assert(Player.RequestPDAMapModeExit(me) == nil)
                assert(Player.RequestPDAMapModeCancel() == nil)
                "#,
            )
            .exec()
            .unwrap();
        assert!(world.borrow().pda_map.is_none());
        host.lua()
            .load(
                r#"
                Player.SetPDAMapMode(me, true, 0, 170, 0, 100)
                Player.RequestPDAMapModeExit(me, function() end)
                Player.RequestPDAMapModeCancel(me)
                "#,
            )
            .exec()
            .unwrap();
        let w = world.borrow();
        let m = w.pda_map.as_ref().unwrap();
        assert!(m.exit.is_some() && m.cancel_asked && m.cancel.is_none());
    }
}
