//! The grappling hook: how far it reaches and how fast it pulls.
//!
//! The heroes' templates (`Hero`, `mattias`, one shared record) carry a
//! `GrappleParameters` component of seven fields. Four are named by their
//! hashes (from samples, by name guesses that match): `Enabled` 1,
//! `MinDistance` 3, `MaxDistance` 65, `ReelSpeed` 10; three are not
//! (`8d473e5f` 7, `e33e547f` 1, `bedb035e` 12). The hook itself (template
//! `Grapple Hook`, the `Grapple` weapon's projectile) flies at its
//! `ControllerVelocity`, 55 in all three of the first fields, and its
//! `TriggerOnTimer` ends it after 10 s.
//!
//! Units are read as metres and metres a second (inferred: 65 m is a
//! helicopter's reach in the game's hint, "get as close to it as you
//! can"). What the game does with the hook (pull her to a helicopter to
//! hijack it, "Helicopters can be hijacked with a grappling hook", the
//! shell's hints) is from the strings, not read from the 360 code.

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};

use crate::tuning::Table;

/// How a hero's grapple reaches and pulls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grapple {
    /// The hero may grapple at all (`Enabled`).
    pub enabled: bool,
    /// Nearer than this the hook is not fired, and the pull ends this
    /// near, metres (`MinDistance`).
    pub min: f32,
    /// The farthest the hook reaches, metres (`MaxDistance`).
    pub max: f32,
    /// How fast she is reeled in, m/s (`ReelSpeed`).
    pub reel: f32,
    /// How fast the hook flies, m/s (the hook's `ControllerVelocity`).
    pub hook_speed: f32,
    /// How long the hook flies before it is given up, seconds (the hook's
    /// `TriggerOnTimer`).
    pub hook_life: f32,
}

impl Default for Grapple {
    /// The values every hero has in the PC data.
    fn default() -> Self {
        Self {
            enabled: true,
            min: 3.0,
            max: 65.0,
            reel: 10.0,
            hook_speed: 55.0,
            hook_life: 10.0,
        }
    }
}

/// The hook's template.
pub const HOOK: &str = "Grapple Hook";

/// Every template's grapple, by id (the heroes'), with the hook's speed and
/// life from [`HOOK`].
pub fn read(db: &Layer<'_>) -> HashMap<u32, Grapple> {
    let params = Table::new(db, "GrappleParameters");
    let velocity = Table::new(db, "ControllerVelocity");
    let timer = Table::new(db, "TriggerOnTimer");
    let names = db.names();
    let hook = names
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(HOOK))
        .map(|(&id, _)| id);
    let base = Grapple::default();
    let hook_speed = hook
        .and_then(|id| {
            velocity
                .of_template(id)
                .first()
                .and_then(|r| velocity.float(r, 0xde4b_900e))
        })
        .unwrap_or(base.hook_speed);
    let hook_life = hook
        .and_then(|id| {
            timer
                .of_template(id)
                .first()
                .and_then(|r| timer.float(r, 0x9469_454a))
        })
        .unwrap_or(base.hook_life);
    let mut out = HashMap::new();
    for id in params.templates() {
        let Some(r) = params.of_template(id).first() else {
            continue;
        };
        out.insert(
            id,
            Grapple {
                enabled: params.word(r, name_hash("Enabled")).is_none_or(|w| w != 0),
                min: params
                    .float(r, name_hash("MinDistance"))
                    .unwrap_or(base.min),
                max: params
                    .float(r, name_hash("MaxDistance"))
                    .unwrap_or(base.max),
                reel: params.float(r, name_hash("ReelSpeed")).unwrap_or(base.reel),
                hook_speed,
                hook_life,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_named_fields_hash_as_the_data_has_them() {
        assert_eq!(name_hash("Enabled"), 0x59a3_e45c);
        assert_eq!(name_hash("MinDistance"), 0x76b5_9d0e);
        assert_eq!(name_hash("MaxDistance"), 0xd2d5_2e54);
        assert_eq!(name_hash("ReelSpeed"), 0x6fbb_7cea);
    }
}
