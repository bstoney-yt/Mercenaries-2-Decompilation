//! A vehicle's `VehiclePart` records: what its exhausts show while it has
//! a driver (a tank's smoke, the heat haze over a light tank's engine).
//!
//! A record's words, by offset: its name at 0x0, its `VehiclePartType` at
//! 0x4, a node of the model at 0x8 and an effect template's name hash at
//! 0xc.
//!
//! # Provenance
//!
//! From samples: the type's names and order are in every layer's enum
//! table (PC data): `__internal`, `LightBrake`, `LightReverse`,
//! `LightFront`, `SpinnyThing`, `LandingGear`, `Exhaust`, `ExhaustStart`,
//! `ExhaustStop`. The `Exhaust` parts of the `Scorpion90`'s 76 templates
//! and some of the `Stingray II`'s name `fx_engineHeatHaze`; others name
//! `global_particle_veh_exhaust_tank` and the like.
//!
//! From the 360 code: `lua_Vehicle_SetParts` and the seat handler
//! (0x822772B0) turn parts on and off by type (0x82276828). Types 6 to 8
//! go to 0x822766F0, which on spawns the effect template the part names
//! (0x82408B88 by name hash, 0x82409B38) at its node, and off stops it.
//! As the driver gets in, 0x822772B0 turns on `ExhaustStart` and
//! `Exhaust`; as they get out, `ExhaustStop` on and `Exhaust` off.
//!
//! From samples: `fx_engineHeatHaze` has an `EffectModel` record whose
//! first field is the name of its effect's one emitter (`e49ad93b`, effect
//! `1b660fc9`), so that emitter's particles are drawn as the model, a flat
//! card 0.38 m wide. Its settings are the data's: 25 a second, living 0.6
//! s, `Size` falling from 1.2 to 0.1.
//!
//! From the 360 code: as an effect starts (0x821D7B10) each emitter looks
//! its name up among the entity's `EffectModel` records and keeps the
//! model (emitter +0x1fc) and the record's flag (+0x207). Each particle of
//! such an emitter is then drawn as the model (0x821DCCE0), scaled evenly
//! by its size at its age; with the flag set, as `fx_engineHeatHaze`'s
//! is, it faces the eye, turned about the view by its spin.
//!
//! Inferred: that the size is what reaches that scale (the decompiled
//! call loses it, and the size sampled just before is the likely one), and
//! that the particles stay at the node (their speed is 0; the 80% of the
//! vehicle's velocity they inherit is left out).
//!
//! From footage: Brendan's co-op video at 7:20, a crewed Puma Light Tank
//! with the air shimmering over its hull. That the Puma is the
//! `Scorpion90` is inferred from its shape.

use std::collections::HashMap;

use m2_assets::effect::{self, Attr, Effect, setting};
use m2_assets::{Layer, name_hash};

use crate::tuning::Table;

/// `VehiclePartType::Exhaust`.
pub const EXHAUST: u32 = 6;

/// The field holding a part's type (name unknown).
const TYPE: u32 = 0x28e1_0525;
/// The field naming its node (name unknown).
const NODE: u32 = 0xab45_283f;
/// The field naming its effect template, by name hash (name unknown).
const EFFECT: u32 = 0xce4a_b1e2;

/// The `EffectModel` field naming its model (hashed as the model asset
/// type is).
const MODEL_FIELD: u32 = 0x5b72_4250;

/// The `EffectModel` field naming the emitter it draws (name unknown).
const EMITTER_FIELD: u32 = 0x4b6b_1847;

/// An `Exhaust` part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exhaust {
    /// The model's node it sits at, by name hash.
    pub node: u32,
    /// The effect template it shows, by name hash.
    pub effect: u32,
}

/// Each template's `Exhaust` parts that name an effect, by template id.
pub fn read(db: &Layer<'_>) -> HashMap<u32, Vec<Exhaust>> {
    let t = Table::new(db, "VehiclePart");
    t.templates()
        .map(|id| {
            let parts: Vec<Exhaust> = t
                .of_template(id)
                .iter()
                .filter(|r| t.word(r, TYPE) == Some(EXHAUST))
                .filter_map(|r| {
                    let effect = t.word(r, EFFECT).filter(|&e| e != 0)?;
                    Some(Exhaust {
                        node: t.word(r, NODE)?,
                        effect,
                    })
                })
                .collect();
            (id, parts)
        })
        .filter(|(_, parts)| !parts.is_empty())
        .collect()
}

/// An emitter whose particles are drawn as a model.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelEmitter {
    pub model: u32,
    /// Its size, keyed over a particle's life (0 to 99).
    pub size: Attr,
    pub size_var: f32,
    pub life: f32,
    pub life_var: f32,
    /// Particles a second.
    pub rate: f32,
}

impl ModelEmitter {
    /// Emitter `name` of `fx`, drawn as `model`.
    pub fn of(fx: &Effect, name: u32, model: u32) -> Option<Self> {
        let e = fx
            .emitters
            .iter()
            .find(|e| effect::find(&e.particle, setting::NAME).is_some_and(|a| a.int() == name))?;
        let value = |s: u32| effect::find(&e.particle, s).map_or(0.0, Attr::float);
        Some(Self {
            model,
            size: effect::find(&e.particle, setting::SIZE)?.clone(),
            size_var: value(setting::SIZE_VAR),
            life: value(setting::LIFE),
            life_var: value(setting::LIFE_VAR),
            rate: value(setting::RATE),
        })
    }

    /// The particles alive `seconds` in, for an emitter told apart by
    /// `salt`: each one's size, and its turn in radians (any way round, as
    /// the data's 360 degree spread has it). Worked out from the time
    /// alone, so nothing need be kept between frames.
    pub fn alive(&self, seconds: f32, salt: u32) -> Vec<(f32, f32)> {
        if self.rate <= 0.0 {
            return Vec::new();
        }
        let longest = self.life + self.life_var;
        let last = (seconds * self.rate).floor() as i64;
        let first = last - (longest * self.rate).ceil() as i64;
        (first..=last)
            .filter_map(|k| {
                let draw = |n: u32| draw(k as u32, salt.wrapping_add(n));
                let born = k as f32 / self.rate;
                let life = self.life + self.life_var * (draw(1) * 2.0 - 1.0);
                let age = (seconds - born) / life.max(0.01);
                (0.0..1.0).contains(&age).then(|| {
                    let size = self.size.at(age * 99.0) + self.size_var * draw(2);
                    (size.max(0.0), draw(3) * std::f32::consts::TAU)
                })
            })
            .collect()
    }
}

/// A draw in [0, 1) for particle `k` (invented: any even draw does).
fn draw(k: u32, salt: u32) -> f32 {
    let mut h = k.wrapping_mul(0x9e37_79b1) ^ salt.wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0xc2b2_ae3d);
    h ^= h >> 13;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Each `EffectModel` template's name hash, with the model it draws, the
/// emitter that draws it and the effect (its `RedEffectComponent`'s
/// `Name`) that emitter is in.
pub fn effect_models(db: &Layer<'_>) -> Vec<(u32, u32, u32, u32)> {
    let names = db.names();
    let t = Table::new(db, "EffectModel");
    let fx = crate::particles::template_effects(db);
    t.templates()
        .filter_map(|id| {
            let r = *t.of_template(id).first()?;
            Some((
                name_hash(names.get(&id)?),
                t.word(r, MODEL_FIELD)?,
                t.word(r, EMITTER_FIELD)?,
                *fx.get(&id)?.first()?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn haze() -> ModelEmitter {
        ModelEmitter {
            model: 0x99ee_ab67,
            size: Attr {
                name: Some(setting::SIZE),
                kind: 1,
                flags: 0,
                value: 1.2f32.to_le_bytes(),
                keys: Vec::new(),
            },
            size_var: 0.1,
            life: 0.6,
            life_var: 0.1,
            rate: 25.0,
        }
    }

    #[test]
    fn about_rate_times_life_are_alive_at_once() {
        let n = haze().alive(100.0, 7).len();
        assert!((10..=20).contains(&n), "{n}");
    }

    #[test]
    fn the_same_moment_gives_the_same_particles() {
        assert_eq!(haze().alive(3.21, 7), haze().alive(3.21, 7));
        assert_ne!(haze().alive(3.21, 7), haze().alive(3.21, 8));
    }

    #[test]
    fn no_rate_no_particles() {
        let none = ModelEmitter {
            rate: 0.0,
            ..haze()
        };
        assert!(none.alive(1.0, 0).is_empty());
    }
}
