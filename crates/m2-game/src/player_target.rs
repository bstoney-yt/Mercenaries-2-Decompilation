//! The player's reticle target and boundary death, from the 360 code:
//! `Player.GetTargetUnderReticle` and `Player.IsBoundaryDeath`.
//!
//! - `Player.GetTargetUnderReticle(player)` (0x8233df68) reads the
//!   player's record: the point under the reticle (+0x114, three floats)
//!   and the object there (+0x10c). It returns x, y, z and the object, or
//!   nil in the object's place when there is none, and nil without a guid.
//!   Any guid reads the local player's record (the lookup falls back to
//!   it). The game keeps the record current
//!   ([`ScriptWorld::reticle_target`]). Which hits the 360's reticle code
//!   counts as the target, and how far it looks, isn't traced: the game's
//!   own crosshair answers here.
//! - `Player.IsBoundaryDeath(character)` (0x8233d9a0) finds the player whose
//!   character it is (0x82467b70; any other guid answers nil). It is true
//!   when the player's boundary sequence has reached its kill (record +0x4a
//!   = 4, set by 0x8243dec0's timers), or when the character is dead
//!   (0x82439cf0: health 0.01 or less) while out of bounds (the controller's
//!   flag +0x4e9, set by 0x8243daf0). The kill phase isn't modelled here, so
//!   it is the second case: dead while out of the play area.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// The health at or below which 0x82439cf0 counts a character as dead.
pub const DEAD_HEALTH: f32 = 0.01;

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let player = lib("Player")?;
    add(&player, "GetTargetUnderReticle", get_target_under_reticle)?;
    add(&player, "IsBoundaryDeath", is_boundary_death)?;
    Ok(())
}

fn guid(v: Option<&Value>) -> Option<u32> {
    crate::events::as_guid(v?)
}

fn get_target_under_reticle(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if guid(a.first()).is_none() {
        return Ok(MultiValue::from_vec(vec![Value::Nil]));
    }
    let (at, target) = w.reticle_target.unwrap_or(([0.0; 3], None));
    let mut out: Vec<Value> = at.iter().map(|&c| Value::Number(f64::from(c))).collect();
    out.push(target.map_or(Value::Nil, crate::events::guid_value));
    Ok(MultiValue::from_vec(out))
}

fn is_boundary_death(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let answer = match guid(a.first()).filter(|&g| Some(g) == w.player) {
        Some(g) => {
            let dead = w
                .objects
                .get(&g)
                .is_none_or(|o| !o.alive || o.health <= DEAD_HEALTH);
            Value::Boolean(dead && w.outside)
        }
        None => Value::Nil,
    };
    Ok(MultiValue::from_vec(vec![answer]))
}

#[cfg(test)]
mod tests {
    use crate::api::{Object, Shared, install};
    use crate::script::ScriptHost;
    use mlua::Value;

    fn setup() -> (ScriptHost, Shared) {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("Mattias", [0.0; 3]));
            w.player = Some(1);
            w.insert(7, Object::new("guard", [3.0, 0.0, 4.0]));
        }
        install(host.lua(), &world).unwrap();
        (host, world)
    }

    fn got(host: &ScriptHost, code: &str) -> Vec<Value> {
        host.lua()
            .load(format!("return {code}"))
            .eval::<mlua::MultiValue>()
            .unwrap()
            .into_iter()
            .collect()
    }

    #[test]
    fn the_reticle_target_is_its_point_and_object() {
        let (host, world) = setup();
        let me = "Player.GetLocalPlayer()";
        let call = format!("Player.GetTargetUnderReticle({me})");
        // Nothing under it yet: the record's zeros and nil.
        assert_eq!(
            got(&host, &call),
            [
                Value::Number(0.0),
                Value::Number(0.0),
                Value::Number(0.0),
                Value::Nil
            ]
        );
        world.borrow_mut().reticle_target = Some(([3.0, 1.5, 4.0], Some(7)));
        let v = got(&host, &call);
        assert_eq!(
            v[..3],
            [Value::Number(3.0), Value::Number(1.5), Value::Number(4.0)]
        );
        let guard: bool = host
            .lua()
            .load(format!(
                "return select(4, {call}) == Pg.GetGuidByName('guard')"
            ))
            .eval()
            .unwrap();
        assert!(guard);
        assert_eq!(
            got(&host, "Player.GetTargetUnderReticle(nil)"),
            [Value::Nil]
        );
    }

    #[test]
    fn a_boundary_death_is_dying_out_of_bounds() {
        let (host, world) = setup();
        let call = "Player.IsBoundaryDeath(Player.GetLocalPlayer())";
        assert_eq!(got(&host, call), [Value::Boolean(false)]);
        world.borrow_mut().outside = true;
        assert_eq!(got(&host, call), [Value::Boolean(false)]);
        {
            let mut w = world.borrow_mut();
            let o = w.objects.get_mut(&1).unwrap();
            o.health = 0.0;
            o.alive = false;
        }
        assert_eq!(got(&host, call), [Value::Boolean(true)]);
        world.borrow_mut().outside = false;
        assert_eq!(got(&host, call), [Value::Boolean(false)]);
        // Not a player's character: nil.
        assert_eq!(
            got(&host, "Player.IsBoundaryDeath(Pg.GetGuidByName('guard'))"),
            [Value::Nil]
        );
    }
}
