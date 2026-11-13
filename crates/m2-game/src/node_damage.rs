//! Objects' node health: `Object.GetNodeHealth` and
//! `ObjectState.SendDamage`, and what a node's damage does to its object
//! ([`m2_engine::node_health`] has the 360 evidence).
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
//!   arguments. The damage, `share` times the node's most health (node 0:
//!   the object's most health, 0x823F7F50), is a damage event of kind 0
//!   (its flags word 0x80000000), so a node it takes to none takes
//!   `DestroyMsg`.
//! - After a node's damage the object dies if no node of its table on a
//!   shown piece of its model has health left; otherwise it takes the
//!   node's pass-through share, which a `Building` takes only from a node
//!   that had health before the hit (0x82289020, 0x82289650). Node 0's
//!   damage is the object's in full. The object's `Health` keeps to its
//!   most and dies under 0.01 (0x822889F8); its death (0x82289F40) zeroes
//!   it, destroys every node with health left (`DestroyMsg`) and calls
//!   the object script's `OnDeath`.
//!
//! A node counts for the death check while the game hasn't hidden its
//! piece ([`ScriptWorld::hidden_nodes`]: m2-alpha hides a building's
//! part, and the nodes under it, once its `DestroyMsg` breaks it; the
//! island fortress's brick nodes hang under its slices this way).
//!
//! Invented: an object's table is built the first time a native asks for
//! it, from its template's records, rather than when it is placed; the
//! two agree while nothing else damages its nodes. Its most health is
//! then set to its template's `Health` amount (the scripts' objects
//! otherwise stand at 100). A node the game hasn't hidden counts as a
//! shown piece even if its model lacks it (the 360 code would not count
//! it; the model isn't known here).
//! The death is carried out as `Object.Kill`'s ([`Command::Kill`]: the
//! game wrecks the object, the scripts' `OnDeath` is called at the next
//! tick).

use std::collections::HashMap;

use m2_assets::model::DESTROY_MSG;
use m2_engine::node_health::{NodeRecord, NodeTable, ObjectHealth, ObjectHit};
use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{Command, ScriptWorld};

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// The `Building` label (`e6124e8d`): an object with it passes nothing on
/// from a node already at none (0x82289650).
pub const BUILDING: u32 = 0xe612_4e8d;

/// A template's `NodeHealth` records and its `Health` amount.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TemplateNodes {
    pub records: Vec<NodeRecord>,
    /// `Health`'s amount, if the template has one above 0.
    pub health: Option<f32>,
}

/// The template database's `NodeHealth` records, with the template's
/// `Health`, by template name in lower case.
pub fn template_nodes(db: &m2_assets::layer::Layer<'_>) -> HashMap<String, TemplateNodes> {
    let names = db.names();
    let health = m2_engine::durability::read(db);
    m2_engine::node_health::read(db)
        .into_iter()
        .filter_map(|(id, records)| {
            let health = health.get(&id).map(|d| d.health).filter(|&h| h > 0.0);
            let name = names.get(&id)?.to_ascii_lowercase();
            Some((name, TemplateNodes { records, health }))
        })
        .collect()
}

impl ScriptWorld {
    /// The template object `guid` was placed as, or for a spawned object
    /// the one it is named after, in lower case.
    fn template_of(&self, guid: u32) -> Option<String> {
        let t = match self.placed_as.get(&guid) {
            Some(t) => t,
            None => &self.objects.get(&guid)?.name,
        };
        Some(t.to_ascii_lowercase())
    }

    /// An object's node table, built from its template's records the
    /// first time it is asked for (which also sets the object's most
    /// health to its template's `Health`); `None` for an object whose
    /// template has none.
    pub fn node_table(&mut self, guid: u32) -> Option<&mut NodeTable> {
        if !self.node_tables.contains_key(&guid) {
            let template = self.template_of(guid)?;
            let t = self.template_nodes.get(&template)?;
            let (table, health) = (NodeTable::new(&t.records), t.health);
            if let (Some(most), Some(o)) = (health, self.objects.get_mut(&guid)) {
                if o.health >= o.max_health {
                    o.health = most;
                }
                o.max_health = most;
            }
            self.node_tables.insert(guid, table);
        }
        self.node_tables.get_mut(&guid)
    }

    /// Whether `guid` is labelled `Building` (its own labels or its
    /// template's).
    fn is_building(&self, guid: u32) -> bool {
        self.objects
            .get(&guid)
            .is_some_and(|o| o.label_hashes.contains(&BUILDING))
            || self
                .template_of(guid)
                .and_then(|t| self.template_labels.get(&t))
                .is_some_and(|l| l.contains(&BUILDING))
    }

    /// Object `guid`'s `Health` takes `damage` (0x822889F8); at its death
    /// it dies as [`die`](Self::die). Its health after.
    fn object_damage(&mut self, guid: u32, damage: f32) -> f32 {
        let Some(o) = self.objects.get_mut(&guid) else {
            return 0.0;
        };
        if !o.alive {
            return o.health;
        }
        let mut h = ObjectHealth {
            most: o.max_health,
            health: o.health,
        };
        match h.take(damage) {
            ObjectHit::Left(left) => {
                o.health = left;
                left
            }
            ObjectHit::Untouched => o.health,
            ObjectHit::Died => {
                self.die(guid);
                0.0
            }
        }
    }

    /// Object `guid` dies of its damage (0x82289F40): its health goes to
    /// 0, every node with health left takes `DestroyMsg` and goes to none,
    /// and it is killed as `Object.Kill` kills.
    fn die(&mut self, guid: u32) {
        let Some(o) = self.objects.get_mut(&guid) else {
            return;
        };
        if !o.alive {
            return;
        }
        o.alive = false;
        o.health = 0.0;
        let gone = self
            .node_tables
            .get_mut(&guid)
            .map(NodeTable::destroy_all)
            .unwrap_or_default();
        for node in gone {
            self.commands.push(Command::NodeDamage {
                guid,
                node,
                damage: 0.0,
                health: Some(0),
                message: Some(DESTROY_MSG),
                pass_through: 0.0,
                object_health: 0.0,
            });
        }
        self.script_killed.push(guid);
        self.commands.push(Command::Kill(guid));
    }

    /// Node `node` of object `guid` takes `damage` of damage kind 0
    /// (0x82289020): the node's message, then the object's death check,
    /// then the pass-through share to its `Health`. Node 0 is the object
    /// itself, which takes it all. Nothing for an object that is dead or
    /// has no such node.
    pub fn damage_node(&mut self, guid: u32, node: u32, damage: f32) {
        if !self.objects.get(&guid).is_some_and(|o| o.alive) {
            return;
        }
        if node == 0 {
            let index = self.commands.len();
            let left = self.object_damage(guid, damage);
            self.commands.insert(
                index,
                Command::NodeDamage {
                    guid,
                    node,
                    damage,
                    health: None,
                    message: None,
                    pass_through: damage,
                    object_health: left,
                },
            );
            return;
        }
        let building = self.is_building(guid);
        if self.node_table(guid).is_none() {
            return;
        }
        let Some(table) = self.node_tables.get_mut(&guid) else {
            return;
        };
        let Some(hit) = table.damage(node, damage, 0, false, building) else {
            return;
        };
        let hidden = self.hidden_nodes.get(&guid);
        let lives = table.any_left(|n| !hidden.is_some_and(|h| h.contains(&n)));
        let index = self.commands.len();
        let left = if !lives {
            // All the health it has left.
            let health = self.objects.get(&guid).map_or(0.0, |o| o.health);
            self.object_damage(guid, health)
        } else if hit.pass_through > 0.0 {
            self.object_damage(guid, hit.pass_through)
        } else {
            self.objects.get(&guid).map_or(0.0, |o| o.health)
        };
        // The node's own message goes before what its object's death
        // sends.
        self.commands.insert(
            index,
            Command::NodeDamage {
                guid,
                node,
                damage,
                health: Some(hit.health),
                message: hit.message,
                pass_through: if lives { hit.pass_through } else { 0.0 },
                object_health: left,
            },
        );
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
    let damage = if node == 0 {
        // Built first, the table sets the most health.
        let _ = w.node_table(guid);
        w.objects.get(&guid).map_or(0.0, |o| o.max_health) * share
    } else {
        match w.node_table(guid).and_then(|t| t.send_damage(node, share)) {
            Some(d) => d,
            None => return Ok(MultiValue::new()),
        }
    };
    if damage > 0.0 {
        w.damage_node(guid, node, damage);
    }
    Ok(MultiValue::new())
}

#[cfg(test)]
mod tests {
    use super::{BUILDING, TemplateNodes};
    use crate::api::{Command, Object, Shared, install};
    use crate::script::ScriptHost;
    use m2_assets::model::DESTROY_MSG;
    use m2_engine::node_health::NodeRecord;
    use mlua::Value;

    fn rec(name: &str, health: f32, pass_through: f32) -> NodeRecord {
        NodeRecord {
            node: m2_assets::name_hash(name),
            health,
            pass_through,
            threshold: 0.0,
        }
    }

    fn host_with(records: Vec<NodeRecord>, health: Option<f32>) -> (ScriptHost, Shared) {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        install(host.lua(), &world).unwrap();
        {
            let mut w = world.borrow_mut();
            w.objects.insert(0x40, Object::new("Fortress", [0.0; 3]));
            w.template_nodes
                .insert("fortress".into(), TemplateNodes { records, health });
        }
        (host, world)
    }

    fn host() -> (ScriptHost, Shared) {
        host_with(vec![rec("Slice2B", 250.0, 0.0)], None)
    }

    /// The `NodeDamage` commands sent: (node, damage, message, object's
    /// health after).
    fn sent(world: &Shared) -> Vec<(u32, f32, Option<u32>, f32)> {
        world
            .borrow()
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::NodeDamage {
                    node,
                    damage,
                    message,
                    object_health,
                    ..
                } => Some((*node, *damage, *message, *object_health)),
                _ => None,
            })
            .collect()
    }

    fn killed(world: &Shared) -> bool {
        world.borrow().commands.contains(&Command::Kill(0x40))
    }

    // The island fortress's KillNode: GetNodeHealth by name, then
    // SendDamage(o, String.GetHash(name), 1) while it has health.
    #[test]
    fn the_fortress_script_kills_a_slice() {
        let (host, world) = host_with(
            vec![rec("Slice2B", 250.0, 0.0), rec("Slice2C", 250.0, 0.0)],
            None,
        );
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
        let slice = m2_assets::name_hash("Slice2B");
        assert_eq!(sent(&world), [(slice, 250.0, Some(DESTROY_MSG), 100.0)]);
        assert!(!killed(&world));
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
        assert!(sent(&world).is_empty());
    }

    // The object's Health is its template's; a node passes its share on.
    #[test]
    fn pass_through_takes_the_objects_health() {
        let (host, world) = host_with(
            vec![rec("Slice2B", 100.0, 1.0), rec("Slice2C", 100.0, 1.0)],
            Some(4000.0),
        );
        host.lua()
            .load(
                "local o = Sys.StringToGuid('0x40')
                 ObjectState.SendDamage(o, String.GetHash('Slice2B'), 0.5)
                 __h = Object.GetHealth(o)",
            )
            .exec()
            .unwrap();
        // 50 damage, a share of 255 * 0.004 = 1.02 of it.
        let h = host.lua().globals().get::<f64>("__h").unwrap();
        assert!((h - (4000.0 - 51.0)).abs() < 1e-3, "{h}");
        let w = world.borrow();
        assert_eq!(w.objects[&0x40].max_health, 4000.0);
        assert!(w.objects[&0x40].alive);
    }

    // Once no node has health, the whole object dies: its Health goes to
    // 0, its other nodes (none left here) are destroyed, it is killed.
    #[test]
    fn the_last_node_kills_the_whole_object() {
        let (host, world) = host_with(
            vec![rec("Slice2B", 100.0, 0.0), rec("Slice2C", 100.0, 0.0)],
            Some(4000.0),
        );
        let lua = host.lua();
        lua.load(
            "local o = Sys.StringToGuid('0x40')
             ObjectState.SendDamage(o, String.GetHash('Slice2B'), 1)
             __alive1 = Object.IsAlive(o)
             ObjectState.SendDamage(o, String.GetHash('Slice2C'), 1)
             __alive2 = Object.IsAlive(o)
             __h = Object.GetHealth(o)",
        )
        .exec()
        .unwrap();
        let g = lua.globals();
        assert!(g.get::<bool>("__alive1").unwrap());
        assert!(!g.get::<bool>("__alive2").unwrap());
        assert_eq!(g.get::<f64>("__h").unwrap(), 0.0);
        let (b, c) = (
            m2_assets::name_hash("Slice2B"),
            m2_assets::name_hash("Slice2C"),
        );
        assert_eq!(
            sent(&world),
            [
                (b, 100.0, Some(DESTROY_MSG), 4000.0),
                (c, 100.0, Some(DESTROY_MSG), 0.0)
            ]
        );
        assert!(killed(&world));
        assert_eq!(world.borrow().script_killed, [0x40]);
        // Dead: more damage does nothing.
        let before = world.borrow().commands.len();
        world.borrow_mut().damage_node(0x40, 0, 10.0);
        assert_eq!(world.borrow().commands.len(), before);
    }

    // The object's own death destroys every node with health left.
    #[test]
    fn the_objects_death_destroys_its_nodes() {
        let (host, world) = host_with(
            vec![rec("Slice2B", 100.0, 0.0), rec("Slice2C", 100.0, 0.0)],
            Some(500.0),
        );
        // Node 0 (a light userdata 0 to the native): all its most health.
        let _ = host;
        let most = {
            let mut w = world.borrow_mut();
            w.node_table(0x40);
            w.objects[&0x40].max_health
        };
        world.borrow_mut().damage_node(0x40, 0, most);
        let (b, c) = (
            m2_assets::name_hash("Slice2B"),
            m2_assets::name_hash("Slice2C"),
        );
        assert_eq!(
            sent(&world),
            [
                (0, 500.0, None, 0.0),
                (b, 0.0, Some(DESTROY_MSG), 0.0),
                (c, 0.0, Some(DESTROY_MSG), 0.0)
            ]
        );
        assert!(killed(&world));
    }

    // A node whose piece the game hid keeps nothing alive.
    #[test]
    fn a_hidden_node_keeps_nothing_alive() {
        let (host, world) = host_with(
            vec![rec("Slice2B", 100.0, 0.0), rec("Brick", 75.0, 0.0)],
            Some(4000.0),
        );
        world
            .borrow_mut()
            .hidden_nodes
            .entry(0x40)
            .or_default()
            .insert(m2_assets::name_hash("Brick"));
        host.lua()
            .load("ObjectState.SendDamage(Sys.StringToGuid('0x40'), String.GetHash('Slice2B'), 1)")
            .exec()
            .unwrap();
        assert!(killed(&world));
        // The brick, with health left, is destroyed by the death.
        let brick = m2_assets::name_hash("Brick");
        assert!(sent(&world).contains(&(brick, 0.0, Some(DESTROY_MSG), 0.0)));
    }

    // A building passes nothing on from a node already at none.
    #[test]
    fn a_building_takes_nothing_from_a_dead_node() {
        let (host, world) = host_with(
            vec![rec("Slice2B", 100.0, 1.0), rec("Slice2C", 100.0, 1.0)],
            Some(4000.0),
        );
        world
            .borrow_mut()
            .template_labels
            .insert("fortress".into(), vec![BUILDING]);
        host.lua()
            .load(
                "local o = Sys.StringToGuid('0x40')
                 ObjectState.SendDamage(o, String.GetHash('Slice2B'), 1)",
            )
            .exec()
            .unwrap();
        let h1 = world.borrow().objects[&0x40].health;
        assert!((h1 - 3898.0).abs() < 1e-3, "{h1}");
        // Slice2B is at none: send_damage still deals its most health,
        // but nothing passes on.
        world
            .borrow_mut()
            .damage_node(0x40, m2_assets::name_hash("Slice2B"), 100.0);
        assert_eq!(world.borrow().objects[&0x40].health, h1);
    }
}
