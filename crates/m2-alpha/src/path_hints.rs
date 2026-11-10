//! The `AiHintNode` points the island's buildings offer an AI vehicle's
//! path search ([`m2_engine::path_search::read_hints`]): where each
//! template's hints are on its model, read once, and where they stand in
//! the world for the body a search met.
//!
//! From the 360 code: the search, blocked by a body, adds a node at each
//! of the body's entity's `AiHintNode` hardpoints (0x822E74A8 walks store
//! 0x83653B38 for the entity; 0x8241E3D0 gives the hardpoint's place in
//! the world). From samples: 20 templates have them (outposts' helipads,
//! alarm towers, bunkers, warehouses and government buildings, two
//! bridges' AI collision, a port plateau), each on a node of its model.
//!
//! Not yet: a merged batch's buildings ([`StaticKey::Member`]) offer none.

use std::collections::HashMap;

use m2_assets::Model;
use m2_engine::Install;
use m2_engine::physics::StaticKey;
use m2_engine::world::Templates;
use m2_math::glam::{Mat4, Quat, Vec3};

/// Each hinted template's points on its model (model space), by hint
/// name hash.
#[derive(Debug, Default)]
pub struct PathHints {
    by_template: HashMap<u32, Vec<(u32, Vec3)>>,
    loaded: bool,
}

impl PathHints {
    /// Reads where every hinted template's hints are on its model, once.
    pub fn load(&mut self, install: &Install, db: &Templates) -> bool {
        if self.loaded {
            return false;
        }
        self.loaded = true;
        let Ok(mut source) = install.open_world() else {
            return false;
        };
        for (&template, hints) in &db.ai_hints {
            let Some(model) = db.infos.get(&template).and_then(|i| i.model) else {
                continue;
            };
            let Ok(d) = source.model(model) else {
                continue;
            };
            let Ok(m) = Model::parse(&d.base) else {
                continue;
            };
            let points = m2_engine::path_search::hint_points(&m.nodes, hints);
            if !points.is_empty() {
                self.by_template.insert(template, points);
            }
        }
        eprintln!(
            "path hints: {} templates' hints placed on their models",
            self.by_template.len()
        );
        true
    }

    /// Whether template `template` has hints placed on its model.
    pub fn has(&self, template: u32) -> bool {
        self.by_template.contains_key(&template)
    }

    /// Template `template`'s hints in the world, its model placed at
    /// `position` turned by `rotation`.
    pub fn placed(&self, template: u32, position: Vec3, rotation: Quat) -> Vec<(u32, Vec3)> {
        let pose = Mat4::from_rotation_translation(rotation, position);
        self.by_template
            .get(&template)
            .map(|p| {
                p.iter()
                    .map(|&(h, at)| (h, pose.transform_point3(at)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The hints of what `key` is, by its template and pose as
    /// `placement` gives them (a placed object's, a state layer's prop's).
    pub fn of(
        &self,
        key: StaticKey,
        placement: impl Fn(StaticKey) -> Option<(u32, Vec3, Quat)>,
    ) -> Vec<(u32, Vec3)> {
        if self.by_template.is_empty() {
            return Vec::new();
        }
        placement(key).map_or_else(Vec::new, |(t, p, r)| self.placed(t, p, r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_turn_and_move_with_the_model() {
        let mut h = PathHints::default();
        h.by_template.insert(7, vec![(1, Vec3::new(5.0, 1.0, 0.0))]);
        let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let p = h.placed(7, Vec3::new(100.0, 0.0, 50.0), turned);
        assert_eq!(p.len(), 1);
        assert!(p[0].1.distance(Vec3::new(100.0, 1.0, 45.0)) < 1e-4, "{p:?}");
        assert!(h.placed(8, Vec3::ZERO, Quat::IDENTITY).is_empty());
        let none = |_: StaticKey| None;
        assert!(h.of(StaticKey::Object(0), none).is_empty());
    }
}
