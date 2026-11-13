//! Natives from the gap list ported one by one from the 360 code, each
//! with its function's address: the HUD and briefing switches, the PDA
//! map's callbacks, material animations and the VO system's cinematic
//! mode.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{Command, ScriptWorld};
use crate::events::as_guid;

type Native = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

fn values<I: IntoIterator<Item = Value>>(v: I) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter(v))
}

fn flag(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Native) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let gui = lib("Gui")?;
    // Gui.LoadTexture(name) (0x8231a9f8): asks the GUI to load a texture
    // by its name hash and answers nil, always. The game's HUD loads its
    // own textures here, so only the request is kept.
    add(&gui, "LoadTexture", |w, _, a| {
        if let Some(Value::String(s)) = a.first() {
            w.textures_asked
                .push(m2_assets::name_hash(&s.to_string_lossy()));
        }
        values([Value::Nil])
    })?;
    // Gui.EnablePlayerMarkers(on) (0x8231c470): whether the other players'
    // markers show (a global flag); nothing back, nil without a flag.
    add(&gui, "EnablePlayerMarkers", |w, _, a| {
        let Some(on) = flag(a.first()) else {
            return values([Value::Nil]);
        };
        w.player_markers = Some(on);
        values([])
    })?;

    let player = lib("Player")?;
    // Player.SetPDAMapModeCallback(player, mode, callback) (0x8233c138):
    // the PDA map's callback for one of its two modes (the flag picks
    // which); true, nil for bad arguments.
    add(&player, "SetPDAMapModeCallback", |w, _, a| {
        let (Some(_), Some(mode), Some(Value::Function(f))) =
            (a.first().and_then(as_guid), flag(a.get(1)), a.get(2))
        else {
            return values([Value::Nil]);
        };
        w.pda_map_callbacks[usize::from(mode)] = Some(f.clone());
        values([Value::Boolean(true)])
    })?;

    let object = lib("Object")?;
    // Object.PlayMaterialAnimation(object, name, loop) (0x82332b70): plays
    // a material animation by name (looping unless told not); the answer
    // is whether it started (here: once the object is known), nil for
    // bad arguments.
    add(&object, "PlayMaterialAnimation", |w, _, a| {
        let (Some(guid), Some(Value::String(name))) = (a.first().and_then(as_guid), a.get(1))
        else {
            return values([Value::Nil]);
        };
        let looped = flag(a.get(2)).unwrap_or(true);
        if !w.objects.contains_key(&guid) {
            return values([Value::Boolean(false)]);
        }
        w.commands.push(Command::MaterialAnimation {
            guid,
            name: name.to_string_lossy(),
            looped,
        });
        values([Value::Boolean(true)])
    })?;

    let graphics = lib("Graphics")?;
    // Graphics.SetShadowBaseDistance(metres) (0x82318038): the renderer's
    // shadow base distance; nothing back, nil without a number.
    add(&graphics, "SetShadowBaseDistance", |w, _, a| {
        let d = match a.first() {
            Some(Value::Number(n)) => *n as f32,
            Some(Value::Integer(i)) => *i as f32,
            _ => return values([Value::Nil]),
        };
        w.commands.push(Command::ShadowBaseDistance(d));
        values([])
    })?;

    let net = lib("Net")?;
    // Net.SetBriefingInterior and Net.SetBriefingCheapCinematic
    // (0x8232d5e8, 0x8232da50): tell the other machine about a briefing,
    // only in a multiplayer session as its server; true always.
    for name in ["SetBriefingInterior", "SetBriefingCheapCinematic"] {
        add(&net, name, |_, _, _| values([Value::Boolean(true)]))?;
    }

    let vo = lib("VO")?;
    // VO.SetCinematicMode(on) (0x82349620): the VO system's cinematic
    // mode; true, nil without a flag.
    add(&vo, "SetCinematicMode", |w, _, a| {
        let Some(on) = flag(a.first()) else {
            return values([Value::Nil]);
        };
        w.vo.cinematic = on;
        w.commands.push(Command::VoCinematic(on));
        values([Value::Boolean(true)])
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::api::Command;
    use crate::script::ScriptHost;

    #[test]
    fn ported_hud_briefing_and_vo_natives_follow_the_360_code() {
        let host = ScriptHost::new().unwrap();
        let world: crate::api::Shared = Default::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, crate::api::Object::new("Mattias", [0.0; 3]));
            w.player = Some(1);
            w.insert(50, crate::api::Object::new("Gate", [0.0; 3]));
        }
        crate::api::install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let ask = |code: &str| -> mlua::Value { lua.load(code).eval().unwrap() };
        assert!(ask("return Gui.LoadTexture('hud_icon')").is_nil());
        assert_eq!(
            ask("return select('#', Gui.EnablePlayerMarkers(false))"),
            mlua::Value::Integer(0)
        );
        assert_eq!(
            ask(
                "return Player.SetPDAMapModeCallback(Player.GetLocalPlayer(), true, function() end)"
            ),
            mlua::Value::Boolean(true)
        );
        assert_eq!(
            ask("return Object.PlayMaterialAnimation(50, 'glow')"),
            mlua::Value::Boolean(true)
        );
        assert_eq!(
            ask("return Object.PlayMaterialAnimation(999, 'glow')"),
            mlua::Value::Boolean(false)
        );
        assert_eq!(
            ask("return Net.SetBriefingInterior('hq')"),
            mlua::Value::Boolean(true)
        );
        assert_eq!(
            ask("return VO.SetCinematicMode(true)"),
            mlua::Value::Boolean(true)
        );
        assert!(ask("return VO.SetCinematicMode()").is_nil());
        let w = world.borrow();
        assert_eq!(w.player_markers, Some(false));
        assert!(w.pda_map_callbacks[1].is_some());
        assert_eq!(w.textures_asked, [m2_assets::name_hash("hud_icon")]);
        drop(w);
        let commands = world.borrow_mut().take_commands();
        assert!(commands.contains(&Command::MaterialAnimation {
            guid: 50,
            name: "glow".into(),
            looped: true
        }));
        assert!(commands.contains(&Command::VoCinematic(true)));
    }
}
