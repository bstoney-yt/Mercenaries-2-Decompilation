//! The lights templates carry: an explosion's flash, a fire's glow.
//!
//! # Provenance
//!
//! From the template database (all read from samples; the field names
//! behind the hashes are unread except where said):
//! - `LightObject` (76 records, 52 bytes): `LightType` (2 point, 3 spot),
//!   `Color` (three floats at 4), `Intensity` (at 16; grenade 12, C4 20,
//!   the air strikes' flashes 15 to 20, a lantern 0.5), then two reaches
//!   at 20 and 24 (always the first the smaller: grenade 0 and 10, a huge
//!   explosion 6 and 12, the MOAB's flash 10 and 24, a lantern 1 and 4):
//!   read as the inner reach, within which the light is full, and the
//!   outer, at which it is gone, as the game's pixel shaders light by
//!   `saturate(d * a + b)` per light (shader3.bin, 052). Inferred.
//! - `LightAnimation` (36 records, 44 bytes): four levels at 0 to 12
//!   (1, 1, 1, 1 on every explosion), `MinIntensityScale` and
//!   `MaxIntensityScale` at 16 and 20 (named by hash), four times at 24
//!   to 36 (the explosions' 0.001, 0.2, 0.001, 0.3 to 0.6 s; a grenade's
//!   0.01, 0.05, 0.03, 0.1) and a flag at 40. Read as: up over the first
//!   time, held for the second, down over the fourth (the third, 1 ms on
//!   explosions, is not used). Inferred; a flash a fifth of a second
//!   long matches Brendan's footage of a satellite bomb.
//! - An explosion or fire template reaches its light through its
//!   `PhysicalLink`s ([`links`]): `Explosion (Grenade)` to `Light_grenade`
//!   2 m up, `Explosion (RPG)` to `Light_rpg`, `fx_EmitFlame` to
//!   `Light_fire_medium` (209 links name a light).

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};
use m2_math::glam::Vec3;

use crate::tuning::Table;

/// A template's light: colour with brightness, reach and how it comes
/// and goes.
#[derive(Debug, Clone, PartialEq)]
pub struct LightSpec {
    /// Linear RGB times the intensity.
    pub color: Vec3,
    pub inner: f32,
    pub outer: f32,
    /// Seconds up, held and down; `None` burns steadily (a fire).
    pub envelope: Option<[f32; 3]>,
}

impl LightSpec {
    /// How bright the light is `age` seconds after it came on, 0 to 1.
    pub fn level(&self, age: f32) -> f32 {
        let Some([up, hold, down]) = self.envelope else {
            return 1.0;
        };
        if age < 0.0 {
            0.0
        } else if age < up {
            age / up.max(1e-4)
        } else if age < up + hold {
            1.0
        } else if age < up + hold + down {
            1.0 - (age - up - hold) / down.max(1e-4)
        } else {
            0.0
        }
    }

    /// When it is over, seconds after it came on (`None`: never).
    pub fn ends(&self) -> Option<f32> {
        self.envelope.map(|[a, b, c]| a + b + c)
    }
}

/// Every template with a light, by its id: the explosion or fire that
/// links to one, and the light templates themselves.
pub fn read(db: &Layer<'_>) -> HashMap<u32, LightSpec> {
    let objects = Table::new(db, "LightObject");
    let anims = Table::new(db, "LightAnimation");
    let f = |t: &Table<'_>, r: &[u8], at: usize| -> Option<f32> {
        let _ = t;
        let b = r.get(at..at + 4)?;
        Some(f32::from_bits(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))).filter(|v| v.is_finite())
    };
    let mut lights: HashMap<u32, LightSpec> = HashMap::new();
    for id in objects.templates() {
        let Some(&r) = objects.of_template(id).first() else {
            continue;
        };
        let intensity = objects.float(r, name_hash("Intensity")).unwrap_or(1.0);
        let color = Vec3::new(
            f(&objects, r, 4).unwrap_or(1.0),
            f(&objects, r, 8).unwrap_or(1.0),
            f(&objects, r, 12).unwrap_or(1.0),
        ) * intensity;
        let inner = f(&objects, r, 20).unwrap_or(0.0).max(0.0);
        let outer = f(&objects, r, 24).unwrap_or(0.0).max(inner);
        let envelope = anims.of_template(id).first().and_then(|a| {
            let up = f(&anims, a, 24)?;
            let hold = f(&anims, a, 28)?;
            let down = f(&anims, a, 36)?;
            // The flickering fires' negative times are no envelope.
            (up >= 0.0 && hold >= 0.0 && down >= 0.0).then_some([up, hold, down])
        });
        lights.insert(
            id,
            LightSpec {
                color,
                inner,
                outer,
                envelope,
            },
        );
    }
    lights
}

/// One of a template's `PhysicalLink`s: the template it sets off with
/// it (an effect, a light, a further explosion) and where, metres from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    pub child: u32,
    pub offset: Vec3,
}

/// Every template's `PhysicalLink`s, by its id. A 44-byte record: the
/// child template at 0, a `Hardpoint` (node name hash) at 4, a `Name` at
/// 8, a `Position` at 12: x, y, z, then a turn as a quaternion (x, y, z,
/// w; 0, 0, 0, 1 on the explosions). From samples: `Explosion
/// (Grenade)` links its flash and dirt effects, its frag explosion and
/// `Light_grenade` 2 m up; `Explosion (C4 Primary)` its light 4 m up;
/// `Explosion (Carpet bomb)` its light 7 m along -z. The turn and the
/// hardpoint are not used here.
pub fn links(db: &Layer<'_>) -> HashMap<u32, Vec<Link>> {
    let t = Table::new(db, "PhysicalLink");
    let f = |r: &[u8], at: usize| -> f32 {
        r.get(at..at + 4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
    };
    t.templates()
        .map(|id| {
            let links = t
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let child = u32::from_le_bytes(r.get(..4)?.try_into().ok()?);
                    (child != 0).then(|| Link {
                        child,
                        offset: Vec3::new(f(r, 12), f(r, 16), f(r, 20)),
                    })
                })
                .collect();
            (id, links)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_rises_holds_and_fades_and_a_fire_burns_on() {
        let flash = LightSpec {
            color: Vec3::ONE,
            inner: 0.0,
            outer: 10.0,
            envelope: Some([0.1, 0.2, 0.4]),
        };
        assert_eq!(flash.level(-1.0), 0.0);
        assert!((flash.level(0.05) - 0.5).abs() < 1e-5);
        assert_eq!(flash.level(0.2), 1.0);
        assert!((flash.level(0.5) - 0.5).abs() < 1e-5);
        assert_eq!(flash.level(0.71), 0.0);
        assert!((flash.ends().unwrap() - 0.7).abs() < 1e-5);
        let fire = LightSpec {
            envelope: None,
            ..flash
        };
        assert_eq!(fire.level(100.0), 1.0);
        assert_eq!(fire.ends(), None);
    }
}
