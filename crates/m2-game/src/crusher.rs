//! The template database's `Crusher` component: what crushes when it
//! falls on someone.
//!
//! From the 360 code: the component's table is 0x83655CC8 (registered by
//! 0x823BF880 under the name `Crusher`). The player's health change
//! (0x822889F8) looks the attacker up in it: a hit from an object that
//! has one is not held by the health clamp and does not start survival
//! mode, so it can kill her outright.
//!
//! From samples: the templates carrying it are falling debris (the
//! `CrushingDebris*` templates, `DebrisDestructTemplate`,
//! `TowerDestructTemplate`, `PropDestructTemplate`, the `debris_*` metal and
//! stone pieces, `global_env_tree_large_debris`, the chandelier). The record
//! has one float (field hash 0x1CB6F279; 500, 1000 or 2000 in the data)
//! whose use was not traced.

use std::collections::HashMap;

use crate::api::ScriptWorld;

/// The templates carrying `Crusher`, by name in lower case, with the
/// record's one value.
pub fn template_crushers(db: &m2_assets::layer::Layer<'_>) -> HashMap<String, f32> {
    let Some(records) = db.component("Crusher").and_then(|c| c.records()) else {
        return HashMap::new();
    };
    let names = db.names();
    records
        .into_iter()
        .filter_map(|(id, r)| {
            let v = f32::from_le_bytes(r.get(..4)?.try_into().ok()?);
            Some((names.get(&id)?.to_ascii_lowercase(), v))
        })
        .collect()
}

impl ScriptWorld {
    /// Whether an object is a crusher: its template (the one it was
    /// placed as, or for a spawned object the one it is named after)
    /// carries `Crusher`.
    pub fn is_crusher(&self, guid: u32) -> bool {
        let template = match self.placed_as.get(&guid) {
            Some(t) => t.clone(),
            None => match self.objects.get(&guid) {
                Some(o) => o.name.clone(),
                None => return false,
            },
        };
        self.template_crushers
            .contains_key(&template.to_ascii_lowercase())
    }
}
