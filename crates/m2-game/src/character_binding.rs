//! The natives `MrxPlayer` uses to make, bind and drop players: the player
//! records, the joined and left callbacks, and `Object.SetModelName`.
//! (`Sys.GetCharacterTemplate` and `VO.CancelAll`, which it also calls,
//! are in `crate::script_shell` and `crate::script_sound`.)
//!
//! From the 360 code (natives 0x8233ea20..0x8233ef48 and 0x8232f140). A
//! player record is found by its number, 0 or 1 (0x82467ab0; none for any
//! other number).
//! - `Player.AttachToCharacter(n, character)` (0x8233eb10, then
//!   0x8243cbc8) makes the character the record's (+0x14), and
//!   `DetachFromCharacter(n)` (0x8233eb88) clears it.
//! - `BindToLocal(n, controller)` (0x8233ec48) first unbinds (0x82438448),
//!   then keeps the controller (+0x1c; 0 when not given).
//! - `BindToRemote(n)` (0x8233ed48) unbinds and marks the record remote
//!   (+0x3c). `Unbind(n)` (0x8233edd8) only unbinds.
//! - `DestroyPlayer(n)` (0x8233ea90, then 0x82437b98) drops the record and
//!   answers true.
//! - `ClearPlayerDB()` (0x8233ea20) empties the records (0x823da7a8 on the
//!   player table 0x83657fa8) and answers true.
//!
//! Each of those, except DestroyPlayer and ClearPlayerDB, answers nil. A
//! missing number answers nil and changes nothing.
//! - `Set/RemovePlayerJoinedCallback` and `Set/RemovePlayerLeftCallback`
//!   (0x8233ee30..0x8233ef48) keep or drop one function each and answer
//!   nil. `MrxPlayer.Start` hands them `OnPlayerJoined` and `OnPlayerLeft`.
//! - `Object.SetModelName(object, name)` (0x8232f140) writes the object's
//!   model name into the table `Player.SetOutfit` also writes (0x836511a4)
//!   and answers true; it reloads nothing itself.
//!
//! The game side: the 360 engine calls the joined callback as a player
//! joins, and `MrxPlayer.OnPlayerJoined` then makes her character and
//! binds it. Here the game makes her itself; with the mission's switch on
//! ([`crate::mission::Mission::set_script_join`], off by default) the
//! joined callback is called for her as well. A record's character doesn't
//! move the game's player ([`crate::api::ScriptWorld::player`]), and
//! nothing calls the left callback yet. Also left out: the
//! detach's 0x8225c728 call on an object flagged 0x10, the controller's
//! input routing (0x8225d288), and what DestroyPlayer frees.

use std::collections::BTreeMap;

use mlua::{Function, Lua, MultiValue, RegistryKey, Table, Value};

use crate::api::ScriptWorld;
use crate::events::as_guid;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// One player record, as the natives above keep it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerRecord {
    /// The character the player controls (+0x14).
    pub character: Option<u32>,
    /// The local controller it is bound to (+0x1c).
    pub controller: Option<u32>,
    /// Bound to a remote machine (+0x3c).
    pub remote: bool,
}

/// What the natives keep, in the script host's app data.
#[derive(Debug, Default)]
pub struct Bindings {
    /// The records by player number (0 or 1).
    pub records: BTreeMap<u32, PlayerRecord>,
    /// Object model names set by `Object.SetModelName`.
    pub model_names: BTreeMap<u32, String>,
    joined: Option<RegistryKey>,
    left: Option<RegistryKey>,
}

impl Bindings {
    /// The function `Player.SetPlayerJoinedCallback` kept, if any.
    pub fn joined(&self, lua: &Lua) -> Option<Function> {
        self.joined
            .as_ref()
            .and_then(|k| lua.registry_value(k).ok())
    }

    /// The function `Player.SetPlayerLeftCallback` kept, if any.
    pub fn left(&self, lua: &Lua) -> Option<Function> {
        self.left.as_ref().and_then(|k| lua.registry_value(k).ok())
    }
}

/// The function `Player.SetPlayerJoinedCallback` kept, if any.
pub fn joined_callback(lua: &Lua) -> Option<Function> {
    lua.app_data_ref::<Bindings>().and_then(|b| b.joined(lua))
}

/// A copy of what the natives keep: the records and the model names.
pub fn records(lua: &Lua) -> (BTreeMap<u32, PlayerRecord>, BTreeMap<u32, String>) {
    lua.app_data_ref::<Bindings>()
        .map(|b| (b.records.clone(), b.model_names.clone()))
        .unwrap_or_default()
}

fn values<const N: usize>(v: [Value; N]) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter(v))
}

fn nil() -> mlua::Result<MultiValue> {
    values([Value::Nil])
}

/// The player number reader (0x82308320): a number, taken whole.
fn number(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Integer(i) => Some(*i as f64 as i64),
        Value::Number(n) => Some(*n as i64),
        _ => None,
    }
}

/// The record of player `n` (0x82467ab0: 0 or 1), made on first use.
fn with_record(lua: &Lua, n: Option<i64>, f: impl FnOnce(&mut PlayerRecord)) {
    let Some(n) = n.and_then(|n| u32::try_from(n).ok()).filter(|&n| n < 2) else {
        return;
    };
    let mut b = bindings(lua);
    f(b.records.entry(n).or_default());
}

fn bindings(lua: &Lua) -> mlua::AppDataRefMut<'_, Bindings> {
    if lua.app_data_ref::<Bindings>().is_none() {
        lua.set_app_data(Bindings::default());
    }
    lua.app_data_mut::<Bindings>().expect("set above")
}

/// Unbinding (0x82438448): no controller, not remote.
fn unbind(r: &mut PlayerRecord) {
    r.controller = None;
    r.remote = false;
}

fn attach_to_character(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if let (Some(n), Some(g)) = (number(a.first()), a.get(1).and_then(as_guid)) {
        with_record(lua, Some(n), |r| r.character = Some(g).filter(|&g| g != 0));
    }
    nil()
}

fn detach_from_character(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    with_record(lua, number(a.first()), |r| r.character = None);
    nil()
}

fn bind_to_local(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let controller = number(a.get(1)).map_or(0, |c| c as u32);
    with_record(lua, number(a.first()), |r| {
        unbind(r);
        r.controller = Some(controller);
    });
    nil()
}

fn bind_to_remote(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    with_record(lua, number(a.first()), |r| {
        unbind(r);
        r.remote = true;
    });
    nil()
}

fn unbind_player(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    with_record(lua, number(a.first()), unbind);
    nil()
}

fn destroy_player(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(n) = number(a.first()) else {
        return nil();
    };
    if let Ok(n) = u32::try_from(n) {
        bindings(lua).records.remove(&n);
    }
    values([Value::Boolean(true)])
}

fn clear_player_db(_: &mut ScriptWorld, lua: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    bindings(lua).records.clear();
    values([Value::Boolean(true)])
}

/// The callback reader (0x82308e90): a function, kept in the registry.
fn keep(lua: &Lua, a: &[Value], joined: bool) -> mlua::Result<MultiValue> {
    if let Some(Value::Function(f)) = a.first() {
        let key = lua.create_registry_value(f.clone())?;
        let mut b = bindings(lua);
        let slot = if joined { &mut b.joined } else { &mut b.left };
        *slot = Some(key);
    }
    nil()
}

fn drop_callback(lua: &Lua, joined: bool) -> mlua::Result<MultiValue> {
    let mut b = bindings(lua);
    let slot = if joined { &mut b.joined } else { &mut b.left };
    *slot = None;
    nil()
}

fn set_joined(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    keep(lua, a, true)
}

fn set_left(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    keep(lua, a, false)
}

fn remove_joined(_: &mut ScriptWorld, lua: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    drop_callback(lua, true)
}

fn remove_left(_: &mut ScriptWorld, lua: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    drop_callback(lua, false)
}

fn set_model_name(_: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(g), Some(Value::String(name))) = (a.first().and_then(as_guid), a.get(1)) else {
        return nil();
    };
    let name = name.to_string_lossy();
    bindings(lua).model_names.insert(g, name);
    values([Value::Boolean(true)])
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let player = lib("Player")?;
    let natives: [(&str, Handler); 11] = [
        ("AttachToCharacter", attach_to_character),
        ("DetachFromCharacter", detach_from_character),
        ("BindToLocal", bind_to_local),
        ("BindToRemote", bind_to_remote),
        ("Unbind", unbind_player),
        ("DestroyPlayer", destroy_player),
        ("ClearPlayerDB", clear_player_db),
        ("SetPlayerJoinedCallback", set_joined),
        ("SetPlayerLeftCallback", set_left),
        ("RemovePlayerJoinedCallback", remove_joined),
        ("RemovePlayerLeftCallback", remove_left),
    ];
    for (name, f) in natives {
        add(&player, name, f)?;
    }
    add(&lib("Object")?, "SetModelName", set_model_name)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Shared, install as install_api};
    use crate::script::ScriptHost;

    fn setup() -> (ScriptHost, Shared) {
        let host = ScriptHost::new().unwrap();
        let world: Shared = std::rc::Rc::default();
        install_api(host.lua(), &world).unwrap();
        (host, world)
    }

    /// MrxPlayer's start, join, character change, leave and quit, in the
    /// order its functions make the calls.
    #[test]
    fn mrx_player_binds_and_drops_players_as_the_360_keeps_them() {
        let (host, _world) = setup();
        let lua = host.lua();
        let answers: (Value, Value, Value, Value, Value) = lua
            .load(
                r#"
                function OnPlayerJoined() end
                function OnPlayerLeft() end
                Player.SetPlayerJoinedCallback(OnPlayerJoined)
                Player.SetPlayerLeftCallback(OnPlayerLeft)
                local hero = Sys.StringToGuid("0x10")
                local a = Player.AttachToCharacter(0, hero)
                Player.BindToLocal(0, 2)
                Player.AttachToCharacter(1, Sys.StringToGuid("0x11"))
                Player.BindToRemote(1)
                Player.AttachToCharacter(5, hero)
                local m = Object.SetModelName(hero, "pmc_hum_chris")
                return a, Player.BindToLocal(), m, nil,
                    Player.Unbind(1)
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            answers,
            (
                Value::Nil,
                Value::Nil,
                Value::Boolean(true),
                Value::Nil,
                Value::Nil
            )
        );
        let (kept, names) = records(lua);
        assert_eq!(
            kept.get(&0),
            Some(&PlayerRecord {
                character: Some(0x10),
                controller: Some(2),
                remote: false
            })
        );
        assert_eq!(
            kept.get(&1),
            Some(&PlayerRecord {
                character: Some(0x11),
                controller: None,
                remote: false
            })
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(names.get(&0x10).map(String::as_str), Some("pmc_hum_chris"));
        {
            let b = lua.app_data_ref::<Bindings>().unwrap();
            assert!(b.joined(lua).is_some() && b.left(lua).is_some());
        }
        let gone: (Value, Value) = lua
            .load(
                r#"
                Player.DetachFromCharacter(0)
                Player.RemovePlayerJoinedCallback()
                Player.RemovePlayerLeftCallback()
                return Player.DestroyPlayer(1), Player.ClearPlayerDB()
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(gone, (Value::Boolean(true), Value::Boolean(true)));
        assert!(records(lua).0.is_empty());
        let b = lua.app_data_ref::<Bindings>().unwrap();
        assert!(b.joined(lua).is_none() && b.left(lua).is_none());
    }
}
