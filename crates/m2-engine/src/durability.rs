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

/// A blast's reach and damage (`Explosive`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Explosion {
    /// Metres (`radius`).
    pub radius: f32,
    /// At its heart (`Damage`).
    pub damage: f32,
    /// At its edge (`minDamage`).
    pub least: f32,
}

/// Each template's blast, by id: those with `Explosive`.
pub fn explosions(db: &Layer<'_>) -> HashMap<u32, Explosion> {
    let t = Table::new(db, "Explosive");
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
                },
            ))
        })
        .collect()
}
