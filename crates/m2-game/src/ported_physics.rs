//! Object physics and protection natives from the 360 code:
//! `Object.GetPhysicsType` and `Object.SetUnkillable`.
//!
//! `GetPhysicsType` (0x82331e30) names the kind of an object's physics
//! body (its vtable +0xe0) by the table [`PHYSICS_TYPES`]. With no body it
//! asks five component tables in turn (car, tank, helicopter, boat,
//! human), and otherwise answers false. Which body each object has is the
//! game's to say: it fills [`ScriptWorld::physics_kinds`]. Objects the game
//! gives no kind (props and buildings not modelled as bodies here) answer
//! false.
//!
//! `SetUnkillable` (0x82330520) sets or clears a reason bit in the
//! object's first protection word (0x8228c980), with the reason hash
//! mapped to a bit by the table `SetInvincible` and `GetInvincible` share
//! (0x8228c610, [`reason_bit`]). The object is unkillable while any bit is
//! set.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{Command, ScriptWorld};

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// The physics body kinds' names, by kind number (0x82331e30's switch).
pub const PHYSICS_TYPES: [&str; 16] = [
    "prop",
    "human",
    "car",
    "tank",
    "helicopter",
    "terrain",
    "building",
    "usable",
    "jet",
    "winch",
    "boat",
    "road",
    "ragdoll",
    "terrainmesh",
    "debris",
    "lowresterrain",
];

/// The body kind of a vehicle the game drives as `kind` (its spec's kind
/// name: "car", "tank", "helicopter", "boat", "jet"), as the 360 numbers
/// them; `None` for a name it doesn't use.
pub fn vehicle_physics_kind(kind: &str) -> Option<u8> {
    Some(match kind.to_ascii_lowercase().as_str() {
        "car" | "bike" | "truck" => 2,
        "tank" => 3,
        "helicopter" | "heli" => 4,
        "jet" | "plane" => 8,
        "boat" => 10,
        _ => return None,
    })
}

/// The bit a protection reason's hash takes (0x8228c610); any other
/// reason takes bit 1, as 0x0a376192 does.
pub fn reason_bit(hash: u32) -> u16 {
    match hash {
        0x0a37_6192 => 0x1,
        0x06be_1abf => 0x2,
        0xc6e8_afa8 => 0x4,
        0xd124_7bfb => 0x8,
        0x22ff_335f => 0x10,
        0x3af8_4adc => 0x20, // "Fanfare"
        0x04c0_f966 => 0x40, // "Support"
        0xac89_2801 => 0x80, // "Hijack"
        0x4496_f668 => 0x100,
        0x4456_8127 => 0x200, // "MrxState"
        0x7ca6_560d => 0x400,
        _ => 0x1,
    }
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let object = lib("Object")?;
    add(&object, "GetPhysicsType", get_physics_type)?;
    add(&object, "SetUnkillable", set_unkillable)?;
    Ok(())
}

fn guid(v: Option<&Value>) -> Option<u32> {
    crate::events::as_guid(v?)
}

fn hash(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) => Some(m2_assets::name_hash(&s.to_string_lossy())),
        Value::Integer(i) => Some(*i as u32),
        Value::Number(n) if n.is_finite() => Some(*n as i64 as u32),
        _ => None,
    }
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

/// `Object.GetPhysicsType(object)`: its body kind's name, false for none,
/// nil without a guid.
fn get_physics_type(w: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(g) = guid(a.first()) else {
        return one(Value::Nil);
    };
    match w
        .physics_kinds
        .get(&g)
        .and_then(|&k| PHYSICS_TYPES.get(usize::from(k)))
    {
        Some(name) => one(Value::String(lua.create_string(name)?)),
        None => one(Value::Boolean(false)),
    }
}

/// `Object.SetUnkillable(object, on, reason)`: sets (or clears) the
/// reason's bit and returns true; nil without a guid. The game hears when
/// the object starts or stops being unkillable ([`Command::Unkillable`]).
/// Inferred: the switch is the boolean read second; the decompiled call
/// hides which register carries it.
fn set_unkillable(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(g) = guid(a.first()) else {
        return one(Value::Nil);
    };
    let on = matches!(a.get(1), Some(Value::Boolean(true)));
    let reason = match a.get(1) {
        Some(Value::Boolean(_)) => a.get(2),
        other => other,
    };
    let bit = reason_bit(hash(reason).unwrap_or(0));
    let bits = w.unkillable.entry(g).or_default();
    let was = *bits != 0;
    if on {
        *bits |= bit;
    } else {
        *bits &= !bit;
    }
    let now = *bits != 0;
    if now != was {
        w.commands.push(Command::Unkillable { guid: g, on: now });
    }
    one(Value::Boolean(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Object, Shared, install};
    use crate::script::ScriptHost;

    #[test]
    fn physics_types_and_unkillable_reasons_as_the_360() {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("Mattias", [0.0; 3]));
            w.player = Some(1);
            w.insert(5, Object::new("jeep", [0.0; 3]));
            w.insert(6, Object::new("crate", [0.0; 3]));
            w.physics_kinds.insert(1, 1);
            w.physics_kinds
                .insert(5, vehicle_physics_kind("car").unwrap());
        }
        install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let got = |code: &str| -> Value { lua.load(format!("return {code}")).eval().unwrap() };
        let name = |code: &str| -> String { lua.load(format!("return {code}")).eval().unwrap() };
        assert_eq!(
            name("Object.GetPhysicsType(Player.GetLocalPlayer())"),
            "human"
        );
        assert_eq!(
            name("Object.GetPhysicsType(Pg.GetGuidByName('jeep'))"),
            "car"
        );
        assert_eq!(
            got("Object.GetPhysicsType(Pg.GetGuidByName('crate'))"),
            Value::Boolean(false)
        );
        assert_eq!(got("Object.GetPhysicsType(nil)"), Value::Nil);
        assert_eq!(vehicle_physics_kind("boat"), Some(10));
        assert_eq!(vehicle_physics_kind("jet"), Some(8));
        // The support crew's reason ("Support", bit 0x40) and the state's;
        // unkillable until every bit is cleared.
        let me = "Player.GetLocalPlayer()";
        assert_eq!(
            got(&format!("Object.SetUnkillable({me}, true, 'Support')")),
            Value::Boolean(true)
        );
        got(&format!("Object.SetUnkillable({me}, true, 'MrxState')"));
        assert_eq!(world.borrow().unkillable[&1], 0x240);
        got(&format!("Object.SetUnkillable({me}, false, 'Support')"));
        got(&format!("Object.SetUnkillable({me}, false, 'MrxState')"));
        assert_eq!(world.borrow().unkillable[&1], 0);
        let cmds = world.borrow_mut().take_commands();
        assert_eq!(
            cmds,
            [
                Command::Unkillable { guid: 1, on: true },
                Command::Unkillable { guid: 1, on: false }
            ]
        );
        assert_eq!(got("Object.SetUnkillable(nil, true)"), Value::Nil);
        assert_eq!(reason_bit(m2_assets::name_hash("Fanfare")), 0x20);
        assert_eq!(reason_bit(m2_assets::name_hash("Hijack")), 0x80);
    }
}
