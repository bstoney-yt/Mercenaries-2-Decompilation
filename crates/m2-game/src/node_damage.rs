//! Objects' node health: `Object.GetNodeHealth` and
//! `ObjectState.SendDamage` ([`m2_engine::node_health`] has the 360
//! evidence).
//!
//! From the 360 code:
//!
//! - `Object.GetNodeHealth(object, name)` (0x8232E898): the name must be
//!   a string (0x82308518, which hashes it); the answer is the node's
//!   health, 0 for an object without that node. A call without an object
//!   or a name answers nil (0x82206F20).
//! - `ObjectState.SendDamage(object, node, share)` (0x82333F80): the node
//!   is a hash as light userdata (`String.GetHash`, Lua type 2 checked by
//!   0x823089B8), the share a number. It answers nothing; nil for bad
//!   arguments. The damage, `share` times the node's most health, is a
//!   damage event of kind 0 (its flags word 0x80000000), so a node it
//!   takes to none takes `DestroyMsg`.
//!
//! Invented: an object's table is built the first time a native asks for
//! it, from its template's records, rather than when it is placed; the
//! two agree while nothing else damages its nodes. Node 0 (the whole
//! object, which the 360 code damages by `share` of its own health) and
//! the object's share of a node's damage are sent on to the game as
//! [`Command::NodeDamage`], not taken off the scripts' health here.

use std::collections::HashMap;

use m2_engine::node_health::{NodeRecord, NodeTable};
use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{Command, ScriptWorld};

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// The template database's `NodeHealth` records, by template name in lower
/// case.
pub fn template_nodes(db: &m2_assets::layer::Layer<'_>) -> HashMap<String, Vec<NodeRecord>> {
    let names = db.names();
    m2_engine::node_health::read(db)
        .into_iter()
        .filter_map(|(id, r)| Some((names.get(&id)?.to_ascii_lowercase(), r)))
        .collect()
}

impl ScriptWorld {
    /// An object's node table, built from its template's records (the
    /// template it was placed as, or for a spawned object the one it is
    /// named after) the first time it is asked for; `None` for an object
    /// whose template has none.
    pub fn node_table(&mut self, guid: u32) -> Option<&mut NodeTable> {
        if !self.node_tables.contains_key(&guid) {
            let template = match self.placed_as.get(&guid) {
                Some(t) => t.clone(),
                None => self.objects.get(&guid)?.name.clone(),
            };
            let records = self.template_nodes.get(&template.to_ascii_lowercase())?;
            let table = NodeTable::new(records);
            self.node_tables.insert(guid, table);
        }
        self.node_tables.get_mut(&guid)
    }
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    add(&lib("Object")?, "GetNodeHealth", get_node_health)?;
    add(&lib("ObjectState")?, "SendDamage", send_damage)
}

fn nil() -> MultiValue {
    MultiValue::from_vec(vec![Value::Nil])
}

/// `Object.GetNodeHealth(object, name)`.
fn get_node_health(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(g), Some(Value::String(name))) =
        (a.first().and_then(crate::events::as_guid), a.get(1))
    else {
        return Ok(nil());
    };
    let node = crate::script_hash::engine_hash(&name.as_bytes());
    let health = w.node_table(g).map_or(0.0, |t| t.health(node));
    Ok(MultiValue::from_vec(vec![Value::Number(f64::from(health))]))
}

/// `ObjectState.SendDamage(object, node hash, share)`.
fn send_damage(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(guid), Some(Value::LightUserData(node)), Some(share)) = (
        a.first().and_then(crate::events::as_guid),
        a.get(1),
        a.get(2).and_then(|v| match v {
            Value::Number(n) => Some(*n as f32),
            Value::Integer(i) => Some(*i as f32),
            _ => None,
        }),
    ) else {
        return Ok(nil());
    };
    let node = node.0 as usize as u32;
    if node == 0 {
        // The whole object: `share` of its health.
        let health = w.objects.get(&guid).map_or(0.0, |o| o.max_health);
        let damage = health * share;
        if damage > 0.0 {
            w.commands.push(Command::NodeDamage {
                guid,
                node,
                damage,
                health: None,
                message: None,
                pass_through: damage,
            });
        }
        return Ok(MultiValue::new());
    }
    let Some(table) = w.node_table(guid) else {
        return Ok(MultiValue::new());
    };
    let Some(damage) = table.send_damage(node, share) else {
        return Ok(MultiValue::new());
    };
    if let Some(hit) = table.damage(node, damage, 0, false) {
        w.commands.push(Command::NodeDamage {
            guid,
            node,
            damage,
            health: Some(hit.health),
            message: hit.message,
            pass_through: hit.pass_through,
        });
    }
    Ok(MultiValue::new())
}

#[cfg(test)]
mod tests {
    use crate::api::{Command, Object, Shared, install};
    use crate::script::ScriptHost;
    use m2_assets::model::DESTROY_MSG;
    use m2_engine::node_health::NodeRecord;
    use mlua::Value;

    fn host() -> (ScriptHost, Shared) {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        install(host.lua(), &world).unwrap();
        {
            let mut w = world.borrow_mut();
            w.objects.insert(0x40, Object::new("Fortress", [0.0; 3]));
            w.template_nodes.insert(
                "fortress".into(),
                vec![NodeRecord {
                    node: m2_assets::name_hash("Slice2B"),
                    health: 250.0,
                    pass_through: 0.0,
                    threshold: 0.0,
                }],
            );
        }
        (host, world)
    }

    // The island fortress's KillNode: GetNodeHealth by name, then
    // SendDamage(o, String.GetHash(name), 1) while it has health.
    #[test]
    fn the_fortress_script_kills_a_slice() {
        let (host, world) = host();
        let lua = host.lua();
        lua.load(
            "local o = Sys.StringToGuid('0x40')
             __before = Object.GetNodeHealth(o, 'Slice2B')
             __none = Object.GetNodeHealth(o, 'Slice9Z')
             if Object.GetNodeHealth(o, 'Slice2B') > 0 then
                 ObjectState.SendDamage(o, String.GetHash('Slice2B'), 1)
             end
             __after = Object.GetNodeHealth(o, 'Slice2B')
             __bad = Object.GetNodeHealth(o)
             Object.Revive(o)
             __revived = Object.GetNodeHealth(o, 'Slice2B')",
        )
        .exec()
        .unwrap();
        let g = lua.globals();
        assert_eq!(g.get::<f64>("__before").unwrap(), 250.0);
        assert_eq!(g.get::<f64>("__none").unwrap(), 0.0);
        assert_eq!(g.get::<f64>("__after").unwrap(), 0.0);
        assert!(matches!(g.get::<Value>("__bad").unwrap(), Value::Nil));
        assert_eq!(g.get::<f64>("__revived").unwrap(), 250.0);
        let w = world.borrow();
        let sent: Vec<_> = w
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::NodeDamage {
                    guid,
                    damage,
                    message,
                    ..
                } => Some((*guid, *damage, *message)),
                _ => None,
            })
            .collect();
        assert_eq!(sent, [(0x40, 250.0, Some(DESTROY_MSG))]);
    }

    // The node is a hash, not a name; an object without a table takes
    // nothing.
    #[test]
    fn send_damage_takes_a_hash_and_an_object_with_nodes() {
        let (host, world) = host();
        host.lua()
            .load(
                "local o = Sys.StringToGuid('0x40')
                 __r = ObjectState.SendDamage(o, 'Slice2B', 1)
                 ObjectState.SendDamage(Sys.StringToGuid('0x41'), String.GetHash('Slice2B'), 1)",
            )
            .exec()
            .unwrap();
        let g = host.lua().globals();
        assert!(matches!(g.get::<Value>("__r").unwrap(), Value::Nil));
        let w = world.borrow();
        assert!(
            !w.commands
                .iter()
                .any(|c| matches!(c, Command::NodeDamage { .. }))
        );
    }
}
