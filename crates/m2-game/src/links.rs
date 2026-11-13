//! The object link table: which objects are attached to which, as the
//! 360 keeps it (component table 0x83656008).
//!
//! From the 360 code:
//! - The table is the engine's keyed component store (insert 0x823d9e10,
//!   iteration 0x823dab38 / 0x823dac60). Each entry is keyed by the parent
//!   (the index at +0x34), and by the child as its second key (+0x50) and
//!   instance id. An insert for a child the parent already holds replaces
//!   that entry.
//! - `Object.Attach(parent, hardpoint, child)` (0x82332750) inserts the
//!   record { hardpoint hash, child guid, local transform } for the pair.
//! - `Object.GetAttachedObjects(parent)` (0x82332a70) lists the second keys
//!   of the parent's entries, the children, in order (a child of 0 as nil).
//! - `ObjectState.GetLinkGuid(parent, id)` (0x823340e8) returns the second
//!   key (the child) of the first of the parent's entries whose record's
//!   second word (+4) equals `id` (a light userdata hash), or nothing; nil
//!   without both arguments.
//! - 0x821873e8 looks an object up by its second key: what it is attached
//!   to ([`Links::parent_of`]). The AI and events use it to stand a
//!   subject in for what carries it.
//!
//! Not established: the table has 32 writers in the 360 code (riders,
//! carried objects, weapons, hijacks among them), and the data-authored
//! links the scripts ask by name (the oil rig's `hp_snap_oilrig_*`
//! buildings) aren't traced to their writer. Their records' second word
//! is the snap name. For `Object.Attach`'s records it is the child guid.
//! Here only what [`Links::attach`] is told is held.

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// One entry: a parent's record for a child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub parent: u32,
    /// The record's first word: `Object.Attach`'s hardpoint hash.
    pub hardpoint: u32,
    /// The record's second word, what `GetLinkGuid` matches.
    pub key: u32,
    pub child: u32,
}

/// The link table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Links {
    entries: Vec<Link>,
}

impl Links {
    /// Inserts a link, replacing the parent's entry for the same child.
    pub fn insert(&mut self, link: Link) {
        match self
            .entries
            .iter_mut()
            .find(|l| l.parent == link.parent && l.child == link.child)
        {
            Some(l) => *l = link,
            None => self.entries.push(link),
        }
    }

    /// `Object.Attach`'s record: the hardpoint, and the child as both the
    /// record's second word and the second key.
    pub fn attach(&mut self, parent: u32, hardpoint: u32, child: u32) {
        self.insert(Link {
            parent,
            hardpoint,
            key: child,
            child,
        });
    }

    /// Drops the parent's link to `child` (`Object.Detach`, 0x823328b8;
    /// that it takes the entry out of this table is inferred).
    pub fn detach(&mut self, parent: u32, child: u32) {
        self.entries
            .retain(|l| !(l.parent == parent && l.child == child));
    }

    /// Drops every link of `guid`, as parent or as child (a removed or
    /// detached object).
    pub fn remove(&mut self, guid: u32) {
        self.entries.retain(|l| l.parent != guid && l.child != guid);
    }

    /// The parent's children, in the order they were linked.
    pub fn children(&self, parent: u32) -> impl Iterator<Item = u32> + '_ {
        self.entries
            .iter()
            .filter(move |l| l.parent == parent)
            .map(|l| l.child)
    }

    /// The child of the parent's first entry whose second word is `key`.
    pub fn linked(&self, parent: u32, key: u32) -> Option<u32> {
        self.entries
            .iter()
            .find(|l| l.parent == parent && l.key == key)
            .map(|l| l.child)
    }

    /// What `child` is attached to (0x821873e8).
    pub fn parent_of(&self, child: u32) -> Option<u32> {
        self.entries
            .iter()
            .find(|l| l.child == child)
            .map(|l| l.parent)
    }
}

/// A layer's `PhysicalLink` records: (parent entity, (link name hash,
/// child entity)).
///
/// From samples: the component's 44-byte record is the child entity (+0),
/// the link's name hash twice (+4, +8: `hp_snap_oilrig_bld_buildingA` for
/// the PMC002 oil rig's `PmcCon002_OilrigA`) and a transform (+12). A
/// parent has one record per child.
///
/// From the 360 code: instancing an object (0x82408c30) copies each of its
/// link records into the link table, with the child as the second key and
/// the record's first two words as the entry's. So the entry's second word,
/// the one `GetLinkGuid` matches, is the name. The template database's
/// records (a template's child templates, spawned with it) aren't read
/// here.
pub fn physical_links(layer: &m2_assets::layer::Layer<'_>) -> Vec<(u32, (u32, u32))> {
    let Some(records) = layer.component("PhysicalLink").and_then(|c| c.records()) else {
        return Vec::new();
    };
    let word = |r: &[u8], at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(r.get(at..at + 4)?.try_into().ok()?))
    };
    records
        .into_iter()
        .filter_map(|(parent, r)| Some((parent, (word(r, 4)?, word(r, 0)?))))
        .collect()
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    add(&lib("Object")?, "GetAttachedObjects", get_attached_objects)?;
    add(&lib("ObjectState")?, "GetLinkGuid", get_link_guid)?;
    Ok(())
}

fn guid(v: Option<&Value>) -> Option<u32> {
    crate::events::as_guid(v?)
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

/// `Object.GetAttachedObjects(parent)`: the children as a list; nil
/// without a guid.
fn get_attached_objects(w: &mut ScriptWorld, lua: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(p) = guid(a.first()) else {
        return one(Value::Nil);
    };
    let list = lua.create_table()?;
    for (i, c) in w.links.children(p).enumerate() {
        let v = if c == 0 {
            Value::Nil
        } else {
            crate::events::guid_value(c)
        };
        list.raw_set(i + 1, v)?;
    }
    one(Value::Table(list))
}

/// `ObjectState.GetLinkGuid(parent, id)`: the linked child, nothing when
/// none matches, nil without both arguments (`id` is light userdata: a
/// hash from `String.GetHash`, or a guid).
fn get_link_guid(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    // A number too: `String.GetHash` answers one here until #426 makes it
    // light userdata as on the 360.
    let key = match a.get(1) {
        Some(Value::LightUserData(id)) => Some(id.0 as usize as u32),
        Some(Value::Number(n)) if n.is_finite() => Some(*n as i64 as u32),
        Some(Value::Integer(i)) => Some(*i as u32),
        _ => None,
    };
    let (Some(p), Some(key)) = (guid(a.first()), key) else {
        return one(Value::Nil);
    };
    match w.links.linked(p, key) {
        Some(c) => one(crate::events::guid_value(c)),
        None => Ok(MultiValue::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Object, Shared, install};
    use crate::script::ScriptHost;

    #[test]
    fn the_table_keeps_a_parents_children_and_replaces_by_child() {
        let mut l = Links::default();
        l.attach(10, 0xaaaa, 20);
        l.attach(10, 0xbbbb, 21);
        l.attach(10, 0xcccc, 20);
        assert_eq!(l.children(10).collect::<Vec<_>>(), [20, 21]);
        assert_eq!(l.parent_of(21), Some(10));
        assert_eq!(l.linked(10, 21), Some(21));
        l.insert(Link {
            parent: 10,
            hardpoint: 0,
            key: 0x5eed,
            child: 30,
        });
        assert_eq!(l.linked(10, 0x5eed), Some(30));
        l.remove(20);
        assert_eq!(l.children(10).collect::<Vec<_>>(), [21, 30]);
        assert_eq!(l.parent_of(20), None);
    }

    /// The PMC002 oil rig's data-authored links, as its script asks them:
    /// placing the rig makes them, and its buildings are found by their
    /// snap names (values from the layer's `PhysicalLink` records).
    #[test]
    fn a_placed_parent_brings_its_physical_links() {
        use crate::mission::{Entity, GameData, Mission};
        let rig = 0x000b_637b;
        let entity = |guid: u32, name: &str| Entity {
            guid,
            name: name.into(),
            at: [0.0; 3],
            yaw: 0.0,
            region: None,
            labels: Vec::new(),
            script: None,
            script_data: None,
            faction: None,
            path: Vec::new(),
            template: None,
            crew: Vec::new(),
            alarm: false,
        };
        let snap = |n: &str| m2_assets::name_hash(n);
        let mut data = GameData::default();
        data.physical_links.insert(
            rig,
            vec![
                (snap("hp_snap_oilrig_bld_buildingA"), 0x0010_ea85),
                (snap("hp_snap_oilrig_bld_buildingB"), 0x0010_ea86),
                (snap("hp_snap_oilrig_catwalkB"), 0x0010_ea8e),
                (snap("hp_snap_oilrig_scaffold"), 0x0010_eb05),
            ],
        );
        data.layers.insert(
            "pmccon002".into(),
            vec![
                entity(rig, "PMC002 Oilrig"),
                entity(0x0010_ea85, "PmcCon002_OilrigA"),
                entity(0x0010_ea86, "PmcCon002_OilrigB"),
            ],
        );
        let mut m = Mission::new(data, "Jennifer", [0.0; 3]).unwrap();
        assert_eq!(m.preload("pmccon002"), ["pmccon002"]);
        m.tick(0.1);
        let found: bool = m
            .lua()
            .load(
                "local rig = Pg.GetGuidByName('PMC002 Oilrig') \
                 return ObjectState.GetLinkGuid(rig, String.GetHash('hp_snap_oilrig_bld_buildingA')) \
                 == Pg.GetGuidByName('PmcCon002_OilrigA') \
                 and #Object.GetAttachedObjects(rig) == 4",
            )
            .eval()
            .unwrap();
        assert!(found);
        // Unloaded: gone.
        m.run("Pg.UnloadLayer('pmccon002')", "unload").unwrap();
        m.tick(0.1);
        assert_eq!(m.world().borrow().links.children(rig).count(), 0);
    }

    /// `Object.Attach` records into the table and `Object.Detach` takes
    /// it out (an object in play hung off the parent's bone).
    #[test]
    fn attach_and_detach_keep_the_table() {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("truck", [0.0; 3]));
            w.insert(2, Object::new("crate", [0.0; 3]));
        }
        install(host.lua(), &world).unwrap();
        crate::hijack::install(&host, &world).unwrap();
        host.lua()
            .load("Object.Attach(Pg.GetGuidByName('truck'), 'hp_bed', Pg.GetGuidByName('crate'))")
            .exec()
            .unwrap();
        assert_eq!(world.borrow().links.children(1).collect::<Vec<_>>(), [2]);
        assert_eq!(world.borrow().links.parent_of(2), Some(1));
        host.lua()
            .load("Object.Detach(Pg.GetGuidByName('truck'), Pg.GetGuidByName('crate'))")
            .exec()
            .unwrap();
        assert_eq!(world.borrow().links.parent_of(2), None);
    }

    #[test]
    fn the_natives_read_the_table() {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("rig", [0.0; 3]));
            w.insert(2, Object::new("buildingA", [0.0; 3]));
            w.insert(3, Object::new("seat", [0.0; 3]));
            w.links.insert(Link {
                parent: 1,
                hardpoint: 0,
                key: m2_assets::name_hash("hp_snap_oilrig_bld_buildingA"),
                child: 2,
            });
            w.links.attach(1, m2_assets::name_hash("hp_seat"), 3);
        }
        install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let named = |code: &str| -> Option<String> {
            lua.load(format!(
                "local g = {code} return g and Object.GetName and tostring(g)"
            ))
            .eval()
            .unwrap()
        };
        let rig = "Pg.GetGuidByName('rig')";
        // By the snap name's hash, as the oil rig's script asks.
        let a: bool = lua
            .load(format!(
                "return ObjectState.GetLinkGuid({rig}, String.GetHash('hp_snap_oilrig_bld_buildingA')) \
                 == Pg.GetGuidByName('buildingA')"
            ))
            .eval()
            .unwrap();
        assert!(a);
        let n: usize = lua
            .load(format!(
                "return select('#', ObjectState.GetLinkGuid({rig}, String.GetHash('nothing')))"
            ))
            .eval()
            .unwrap();
        assert_eq!(n, 0);
        assert!(named(&format!("ObjectState.GetLinkGuid({rig})")).is_none());
        // Every child, in order.
        let count: usize = lua
            .load(format!("return #Object.GetAttachedObjects({rig})"))
            .eval()
            .unwrap();
        assert_eq!(count, 2);
        let nil: Value = lua
            .load("return Object.GetAttachedObjects(nil)")
            .eval()
            .unwrap();
        assert_eq!(nil, Value::Nil);
    }
}
