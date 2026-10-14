//! What the template database says about how much a placed object takes
//! before it is destroyed, and how a building comes down.
//!
//! Every placed object names its template through its `Transform`'s
//! parent (81,857 of 81,865 placed models, survey `destruct`, round D1).
//! A building's template carries `Health`, `NodeHealth` (per part),
//! `_BuildingPhysics` and `BuildingCollapseAnim`; its model's state
//! machine does the rest (pristine, collapse, debris, the lasting wreck).
//!
//! Field keys without a known name are used as they are; the comments say
//! what each holds, read off the values.

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};

use crate::tuning::Table;

/// `Health`'s amount (name unknown): 100 on 1,006 templates, 300, 25, 50.
const AMOUNT: u32 = 0xd122_919d;
/// `BuildingCollapseAnim`'s fields: how fast it sinks (`SinkAccel`, m/s²,
/// 2.5 on 875 of 880), its fastest sinking (-1: no limit; name unknown),
/// and the least and most it leans as it goes (degrees, 8 and 10 on
/// most; names unknown, read as the gate's 8 to 10).
const SINK_MOST_SPEED: u32 = 0xbdd8_d049;
const LEAN_LEAST: u32 = 0xea7f_5612;
const LEAN_MOST: u32 = 0x23e3_5adc;

/// How much a template's objects take, and how they come down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Durability {
    /// Damage it takes before it is destroyed (`Health`).
    pub health: f32,
    /// Nothing destroys it (`Health`'s `Invincible`).
    pub invincible: bool,
    /// A building's collapse, if it has one.
    pub collapse: Option<Collapse>,
}

/// How a building sinks and leans as it comes down
/// (`BuildingCollapseAnim`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collapse {
    /// `Type` (0 on 742, 1 on 138; meaning unknown).
    pub kind: u32,
    /// m/s².
    pub sink_accel: f32,
    /// m/s, negative: no limit.
    pub sink_most_speed: f32,
    /// Degrees.
    pub lean: (f32, f32),
}

/// Each template's durability, by id: those with `Health`.
pub fn read(db: &Layer<'_>) -> HashMap<u32, Durability> {
    let health = Table::new(db, "Health");
    let anim = Table::new(db, "BuildingCollapseAnim");
    let invincible = name_hash("Invincible");
    let (kind, sink) = (name_hash("Type"), name_hash("SinkAccel"));
    health
        .templates()
        .filter_map(|id| {
            let r = health.of_template(id).first()?;
            let collapse = anim.of_template(id).first().map(|a| Collapse {
                kind: anim.word(a, kind).unwrap_or(0),
                sink_accel: anim.float(a, sink).unwrap_or(2.5),
                sink_most_speed: anim.float(a, SINK_MOST_SPEED).unwrap_or(-1.0),
                lean: (
                    anim.float(a, LEAN_LEAST).unwrap_or(8.0),
                    anim.float(a, LEAN_MOST).unwrap_or(10.0),
                ),
            });
            Some((
                id,
                Durability {
                    health: health.float(r, AMOUNT).unwrap_or(0.0),
                    invincible: health.word(r, invincible) == Some(1),
                    collapse,
                },
            ))
        })
        .collect()
}

/// `PhysicsPropertyCrashable`'s fields (names unknown, read off the
/// values, round D8): how a vehicle running into it breaks it (0 on 238
/// templates, fences among them; 1 on 48, lampposts; 2 on 136, the
/// plants and bushes), and what it takes to (950 on 84, the fences;
/// 29,900 on 26, emplaced guns; 15,000, 3,900; 0 on the rest).
const CRASH_HOW: u32 = 0xaf54_061b;
const CRASH_TAKES: u32 = 0x6b54_d7a7;

/// How a vehicle running into a template's objects breaks them
/// (`PhysicsPropertyCrashable`, on 366 templates and 39,327 placed
/// objects; shacks and other buildings have none and come down only to
/// damage).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crash {
    pub how: CrashHow,
    /// What it takes: read as the vehicle's mass (kg), as the values line
    /// up with the vehicles' (a jeep's 1,000 and a Humvee's 3,400 pass a
    /// fence's 950; only a tank's 32,000 passes 29,900). Inferred.
    pub takes: f32,
}

/// How [`Crash`] breaks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashHow {
    /// Broken by a vehicle heavy enough ([`Crash::takes`]): fences.
    Breaks,
    /// Knocked over: lampposts.
    KnockedOver,
    /// Flattened by any vehicle: plants and bushes.
    Flattened,
}

/// Each template that vehicles break running into, by id.
pub fn crashable(db: &Layer<'_>) -> HashMap<u32, Crash> {
    let crash = Table::new(db, "PhysicsPropertyCrashable");
    crash
        .templates()
        .filter_map(|id| {
            let r = crash.of_template(id).first()?;
            let how = match crash.word(r, CRASH_HOW).unwrap_or(0) {
                1 => CrashHow::KnockedOver,
                2 => CrashHow::Flattened,
                _ => CrashHow::Breaks,
            };
            let takes = crash.float(r, CRASH_TAKES).unwrap_or(0.0).max(0.0);
            Some((id, Crash { how, takes }))
        })
        .collect()
}

/// Each template's `SoundKey`, by id, where it isn't zero: the sound its
/// objects make as they break (`_global_fencewoodpanel`'s `286115a2`,
/// `prop_sheetmetal_sml`; the chain-link fences' `16117a0b`,
/// `prop_metal_chainlinkfence`; PC survey `soundkeys`, 2026-10-09). A
/// template with no `BuildingCollapseAnim` ([`Durability::collapse`])
/// doesn't come down as a building does, with its rumble: it is heard as
/// its own key says.
pub fn sound_keys(db: &Layer<'_>) -> HashMap<u32, u32> {
    let t = Table::new(db, "SoundKey");
    let field = name_hash("SoundKey");
    t.templates()
        .filter_map(|id| {
            let key = t.word(t.of_template(id).first()?, field)?;
            (key != 0).then_some((id, key))
        })
        .collect()
}

/// `_PropPhysics`'s field read as a flag (name unknown; 1 on 102
/// templates' records only for the heavy ones: shipping containers
/// (20,000 kg), dumpsters, fuel and propane tanks (10,000 kg), the
/// explosive barrel, car ruins (1,000 to 2,000 kg), the large cash case;
/// 0 on chairs, benches, planters, boxes, barrels, barricades). PC
/// survey `breakables`, round 6; from samples.
const PROP_HEAVY: u32 = 0xeca7_c282;

/// A template whose objects are loose physics props (`_PropPhysics`,
/// against `_BuildingPhysics` for what stands fixed): chairs, benches,
/// café tables, planters, boxes, barrels, barricades, trash, dumpsters,
/// containers. They move when hit, as their `ModifierKey` row of the
/// damage table says what breaks them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropPhysics {
    /// Kilograms (`Mass`), where it gives one: 0 or -1 on most (the
    /// game's own default then, which the data doesn't give).
    pub mass: Option<f32>,
    /// [`PROP_HEAVY`] set: one of the heavy props.
    pub heavy: bool,
}

/// What a physics prop weighs when its `Mass` doesn't say (0 or -1, on
/// most), kg: the game's own default isn't in the data. Invented: a light
/// prop's 50 (a chair, a crate; the data's own masses run 20 to 200 for
/// such), a heavy one's 2,000 (as the car ruins').
const PROP_KG: f32 = 50.0;
const HEAVY_KG: f32 = 2000.0;

impl PropPhysics {
    /// Its mass, kg: its own, else [`PROP_KG`] or [`HEAVY_KG`].
    pub fn kg(&self) -> f32 {
        self.mass
            .unwrap_or(if self.heavy { HEAVY_KG } else { PROP_KG })
    }
}

/// Each template's `_PropPhysics`, by id.
pub fn prop_physics(db: &Layer<'_>) -> HashMap<u32, PropPhysics> {
    let t = Table::new(db, "_PropPhysics");
    let mass = name_hash("Mass");
    t.templates()
        .filter_map(|id| {
            let r = t.of_template(id).first()?;
            Some((
                id,
                PropPhysics {
                    mass: t.float(r, mass).filter(|m| m.is_finite() && *m > 0.0),
                    heavy: t.word(r, PROP_HEAVY) == Some(1),
                },
            ))
        })
        .collect()
}

/// Each template's `ModifierKey` (its field of that name, a name hash:
/// `PropFragile`, `Prop`, `Fence`, `Shack`, `Car`, `Tank`, ...), by id:
/// the damage table's row for what its objects take of each kind of
/// damage (on 1,083 templates).
pub fn modifier_keys(db: &Layer<'_>) -> HashMap<u32, u32> {
    let t = Table::new(db, "ModifierKey");
    let field = name_hash("ModifierKey");
    t.templates()
        .filter_map(|id| Some((id, t.word(t.of_template(id).first()?, field)?)))
        .collect()
}

/// A blast's reach and damage (`Explosive`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Explosion {
    /// Metres (`radius`).
    pub radius: f32,
    /// At its heart (`Damage`).
    pub damage: f32,
    /// At its edge (`minDamage`).
    pub least: f32,
    /// Its template's `DamageKey` (name hash; 0 for none): the damage
    /// table's column for what each thing it reaches takes of it (the
    /// RPG's blast `Explosion`, its frag `Bullet`, the air strike's bomb
    /// `ExplosionLarge`).
    pub kind: u32,
}

/// Each template's `DamageKey` (name hash), by id, where it isn't 0.
pub fn damage_keys(db: &Layer<'_>) -> HashMap<u32, u32> {
    let t = Table::new(db, "DamageKey");
    let field = name_hash("DamageKey");
    t.templates()
        .filter_map(|id| {
            let k = t.word(t.of_template(id).first()?, field)?;
            (k != 0).then_some((id, k))
        })
        .collect()
}

/// What a projectile does where it lands (`ProjectilePhysics`'s `Damage`)
/// and of which kind (its `DamageKey`): the `RPG Rocket`'s 100 `Rocket`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projectile {
    pub damage: f32,
    pub kind: u32,
}

/// Each template's projectile, by id: those with `ProjectilePhysics`.
pub fn projectiles(db: &Layer<'_>) -> HashMap<u32, Projectile> {
    let t = Table::new(db, "ProjectilePhysics");
    let damage = name_hash("Damage");
    let keys = damage_keys(db);
    t.templates()
        .filter_map(|id| {
            let r = t.of_template(id).first()?;
            Some((
                id,
                Projectile {
                    damage: t.float(r, damage).unwrap_or(0.0),
                    kind: keys.get(&id).copied().unwrap_or(0),
                },
            ))
        })
        .collect()
}

/// Each template's blast, by id: those with `Explosive`.
pub fn explosions(db: &Layer<'_>) -> HashMap<u32, Explosion> {
    let t = Table::new(db, "Explosive");
    let keys = damage_keys(db);
    let (radius, damage, least) = (
        name_hash("radius"),
        name_hash("Damage"),
        name_hash("minDamage"),
    );
    t.templates()
        .filter_map(|id| {
            let r = t.of_template(id).first()?;
            Some((
                id,
                Explosion {
                    radius: t.float(r, radius).unwrap_or(0.0),
                    damage: t.float(r, damage).unwrap_or(0.0),
                    least: t.float(r, least).unwrap_or(0.0),
                    kind: keys.get(&id).copied().unwrap_or(0),
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prop_weighs_its_own_mass_else_a_light_or_heavy_default() {
        let planter = PropPhysics {
            mass: Some(200.0),
            heavy: false,
        };
        assert_eq!(planter.kg(), 200.0);
        let chair = PropPhysics {
            mass: None,
            heavy: false,
        };
        assert_eq!(chair.kg(), PROP_KG);
        let dumpster = PropPhysics {
            mass: None,
            heavy: true,
        };
        assert_eq!(dumpster.kg(), HEAVY_KG);
    }
}
