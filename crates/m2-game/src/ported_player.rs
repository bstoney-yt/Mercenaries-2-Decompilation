//! Gap-list natives ported one by one from the 360 code, each with its
//! function's address: player and camera bookkeeping, a person's ragdoll
//! and transform, listener and boundary effects, material animations and
//! the disposer. Their effects reach the game as [`PortedCommand`]s.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{Command, ScriptWorld};
use crate::events::{as_guid, guid_value};

type Native = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// What these natives ask of the game.
#[derive(Clone, Debug, PartialEq)]
pub enum PortedCommand {
    /// `Player.TeleportCamera`: the camera to her, a metre up.
    CameraToPlayer,
    /// `Player.ClearGPS`: her GPS route cleared.
    ClearGps,
    /// `Human.SetPreemptiveRagdoll`: the person goes limp at the next hit.
    PreemptiveRagdoll(u32),
    /// `Graphics.SetBoundaryEffect(strength)`.
    BoundaryEffect(f32),
    /// `Sound.LockListenerPosition(on)`.
    ListenerLocked(bool),
    /// `Object.StopMaterialAnimation(object, name)`.
    StopMaterialAnimation { guid: u32, name: String },
    /// `Object.AddToDisposer(object, kind)`: cleared away by the disposer
    /// in time; the kind as a name hash.
    Dispose { guid: u32, kind: u32 },
}

fn values<I: IntoIterator<Item = Value>>(v: I) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter(v))
}

fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Number(n) => Some(*n as f32),
        Value::Integer(i) => Some(*i as f32),
        _ => None,
    }
}

fn hash(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) => Some(m2_assets::name_hash(&s.to_string_lossy())),
        Value::Integer(i) => Some(*i as u32),
        _ => None,
    }
}

/// The label the 360 gives an object put on the disposer (0xf956736b).
pub const DISPOSER_LABEL: u32 = 0xf956_736b;

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Native) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let player = lib("Player")?;
    // Player.GetPlayerId(player) (0x8233e408): the player's number; an
    // unknown player reads the local one's (the 360 falls back to its
    // record). One player here, number 0; nil for no guid.
    add(&player, "GetPlayerId", |_, _, a| {
        if a.first().and_then(as_guid).is_none() {
            return values([Value::Nil]);
        }
        values([Value::Integer(0)])
    })?;
    // Player.GetViewportId(player) (0x8233be88): 1 (as light userdata)
    // for a local player with a viewport; no argument reads the local one.
    add(&player, "GetViewportId", |_, _, _| values([guid_value(1)]))?;
    // Player.TeleportCamera(player or character) (0x8233c000): the camera
    // jumps to her place, a metre up; nothing back, nil for no guid.
    add(&player, "TeleportCamera", |w, _, a| {
        if a.first().and_then(as_guid).is_none() {
            return values([Value::Nil]);
        }
        w.commands
            .push(Command::Ported(PortedCommand::CameraToPlayer));
        values([])
    })?;
    // Player.SetWaitForInGame(player) (0x8233f358): the player waits for
    // the game proper before taking part; true, nil for no guid.
    add(&player, "SetWaitForInGame", |w, _, a| {
        if a.first().and_then(as_guid).is_none() {
            return values([Value::Nil]);
        }
        w.wait_for_in_game = true;
        values([Value::Boolean(true)])
    })?;
    // Player.ClearGPS(player) (0x8233ff98): her GPS route cleared; nothing
    // back, nil for no guid.
    add(&player, "ClearGPS", |w, _, a| {
        if a.first().and_then(as_guid).is_none() {
            return values([Value::Nil]);
        }
        w.commands.push(Command::Ported(PortedCommand::ClearGps));
        values([])
    })?;

    let human = lib("Human")?;
    // Human.SetPreemptiveRagdoll(person) (0x82324e30): nothing back.
    add(&human, "SetPreemptiveRagdoll", |w, _, a| {
        let Some(guid) = a.first().and_then(as_guid) else {
            return values([Value::Nil]);
        };
        w.commands
            .push(Command::Ported(PortedCommand::PreemptiveRagdoll(guid)));
        values([])
    })?;
    // Human.PersistTransform(person) (0x82324eb8): the person's place kept
    // as where it starts from; true, nil for no guid.
    add(&human, "PersistTransform", |w, _, a| {
        let Some(guid) = a.first().and_then(as_guid) else {
            return values([Value::Nil]);
        };
        if let Some(o) = w.objects.get(&guid) {
            let kept = (o.position, o.yaw);
            w.persisted.insert(guid, kept);
        }
        values([Value::Boolean(true)])
    })?;

    let graphics = lib("Graphics")?;
    // Graphics.SetBoundaryEffect(strength) (0x8231a860): the out-of-bounds
    // screen effect's strength; nothing back, nil without a number.
    add(&graphics, "SetBoundaryEffect", |w, _, a| {
        let Some(s) = num(a.first()) else {
            return values([Value::Nil]);
        };
        w.commands
            .push(Command::Ported(PortedCommand::BoundaryEffect(s)));
        values([])
    })?;

    let sound = lib("Sound")?;
    // Sound.LockListenerPosition(on) (0x823412d8): nothing back, nil
    // without a flag.
    add(&sound, "LockListenerPosition", |w, _, a| {
        let Some(Value::Boolean(on)) = a.first() else {
            return values([Value::Nil]);
        };
        w.commands
            .push(Command::Ported(PortedCommand::ListenerLocked(*on)));
        values([])
    })?;

    // Camera.SetLookAt is the script camera's (crate::script_camera).

    let object = lib("Object")?;
    // Object.StopMaterialAnimation(object, name) (0x82332d08): stops it if
    // that one plays; nothing back.
    add(&object, "StopMaterialAnimation", |w, _, a| {
        let (Some(guid), Some(Value::String(name))) = (a.first().and_then(as_guid), a.get(1))
        else {
            return values([Value::Nil]);
        };
        w.commands
            .push(Command::Ported(PortedCommand::StopMaterialAnimation {
                guid,
                name: name.to_string_lossy(),
            }));
        values([])
    })?;
    // Object.AddToDisposer(object, kind) (0x82333d28): labelled for the
    // disposer (0xf956736b) and handed to it; nothing back.
    add(&object, "AddToDisposer", |w, _, a| {
        let (Some(guid), Some(kind)) = (a.first().and_then(as_guid), hash(a.get(1))) else {
            return values([Value::Nil]);
        };
        if let Some(o) = w.objects.get_mut(&guid)
            && !o.label_hashes.contains(&DISPOSER_LABEL)
        {
            o.label_hashes.push(DISPOSER_LABEL);
        }
        w.commands
            .push(Command::Ported(PortedCommand::Dispose { guid, kind }));
        values([])
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::ScriptHost;

    #[test]
    fn ported_player_camera_and_object_natives_follow_the_360_code() {
        let host = ScriptHost::new().unwrap();
        let world: crate::api::Shared = Default::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, crate::api::Object::new("Mattias", [1.0, 2.0, 3.0]));
            w.player = Some(1);
            w.insert(50, crate::api::Object::new("Crate", [0.0; 3]));
        }
        crate::api::install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        assert_eq!(
            ask("return Player.GetPlayerId(Player.GetLocalPlayer())"),
            Value::Integer(0)
        );
        assert_eq!(
            ask("return Player.GetViewportId() == Player.GetLocalPlayer()"),
            Value::Boolean(true)
        );
        assert_eq!(
            ask("return Player.SetWaitForInGame(Player.GetLocalPlayer())"),
            Value::Boolean(true)
        );
        assert_eq!(
            ask("return Human.PersistTransform(1)"),
            Value::Boolean(true)
        );
        for code in [
            "Player.TeleportCamera(Player.GetLocalPlayer())",
            "Player.ClearGPS(Player.GetLocalPlayer())",
            "Human.SetPreemptiveRagdoll(50)",
            "Graphics.SetBoundaryEffect(0.5)",
            "Sound.LockListenerPosition(true)",
            "Object.StopMaterialAnimation(50, 'glow')",
            "Object.AddToDisposer(50, 'Vehicle')",
        ] {
            let n: i64 = lua
                .load(format!("return select('#', {code})"))
                .eval()
                .unwrap();
            assert_eq!(n, 0, "{code}");
        }
        let mut w = world.borrow_mut();
        assert!(w.wait_for_in_game);
        assert_eq!(w.persisted.get(&1), Some(&([1.0, 2.0, 3.0], 0.0)));
        assert!(w.objects[&50].label_hashes.contains(&DISPOSER_LABEL));
        let ported: Vec<PortedCommand> = w
            .take_commands()
            .into_iter()
            .filter_map(|c| match c {
                Command::Ported(p) => Some(p),
                _ => None,
            })
            .collect();
        assert!(ported.contains(&PortedCommand::Dispose {
            guid: 50,
            kind: m2_assets::name_hash("Vehicle"),
        }));
        assert_eq!(ported.len(), 7);
    }
}
