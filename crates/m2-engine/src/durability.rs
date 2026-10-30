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
    /// How it burns down once worn low (`TickDamage`), if it does.
    pub burn: Option<BurnDown>,
}

/// How an object worn below a health burns itself down (`TickDamage`:
/// `DamageThresh`, `Damage`, `TickLength`). From the 360 code: damage
/// that carries its health from above `below` to under it starts a timer
/// (0x822889F8) that adds each frame's time and, each time it reaches
/// `every`, has it take `damage` and starts again less `every`
/// (0x8240ADF8), until it is destroyed. `below` is a whole number
/// (`DamageThresh` is read as one, 0x823F4300), and the burning is armed
/// only when both `damage` and `every` are above 0 (0x823FCB98). The fuel
/// tanks': below 465 of their 500, 50 every half second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BurnDown {
    pub below: f32,
    pub damage: f32,
    pub every: f32,
}

impl BurnDown {
    /// Seconds it burns with `health` left until it is destroyed, with
    /// nothing else hurting it: a tick of `damage` each `every`, the first
    /// `every` after it caught.
    pub fn seconds_to_burn_out(&self, health: f32) -> f32 {
        (health / self.damage).ceil() * self.every
    }
}

/// What a blast does `d` metres from its heart: `damage` falling off with
/// the distance to none at `radius`, but never less than `least` (the 360
/// code's 0x82291640: `max(least, damage * clamp(1 - d / radius, 0, 1))`,
/// before the damage table's cell). What lies past `radius` it does not
/// reach.
pub fn blast_damage(damage: f32, least: f32, radius: f32, d: f32) -> f32 {
    if d > radius {
        return 0.0;
    }
    let falloff = (1.0 - d / radius.max(0.01)).clamp(0.0, 1.0);
    (damage * falloff).max(least)
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
    let tick = Table::new(db, "TickDamage");
    let (below, damage, every) = (
        name_hash("DamageThresh"),
        name_hash("Damage"),
        name_hash("TickLength"),
    );
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
                    burn: tick.of_template(id).first().and_then(|t| {
                        let b = BurnDown {
                            below: tick.word(t, below)? as f32,
                            damage: tick.float(t, damage)?,
                            every: tick.float(t, every)?,
                        };
                        (b.damage > 0.0 && b.every > 0.0).then_some(b)
                    }),
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

/// Each template whose `PhysicsPropertyCrashable` records name nodes of
/// its model (their `Name`, a node's name hash): the parts a vehicle
/// breaks one at a time, by id, each named node with how it breaks. A
/// long hedge's sections each have one (`_global_env_hedgelong`: four,
/// flattened); so do a gate's two leaves (`_pmcoutpost_gate`: broken by
/// a vehicle of 950 kg or more). A record naming no node, beside named
/// ones, is left to [`crashable`]. PC data; that a vehicle breaks only
/// the parts it runs into is from the footage (the real game's jeep
/// leaves a hedge's other sections standing, playtest 3, 11:40-12:14).
pub fn crash_parts(db: &Layer<'_>) -> HashMap<u32, Vec<(u32, Crash)>> {
    let crash = Table::new(db, "PhysicsPropertyCrashable");
    let name = name_hash("Name");
    crash
        .templates()
        .filter_map(|id| {
            let parts: Vec<(u32, Crash)> = crash
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let node = crash.word(r, name).filter(|&n| n != 0)?;
                    let how = match crash.word(r, CRASH_HOW).unwrap_or(0) {
                        1 => CrashHow::KnockedOver,
                        2 => CrashHow::Flattened,
                        _ => CrashHow::Breaks,
                    };
                    let takes = crash.float(r, CRASH_TAKES).unwrap_or(0.0).max(0.0);
                    Some((node, Crash { how, takes }))
                })
                .collect();
            (!parts.is_empty()).then_some((id, parts))
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

/// `OSMParameter`'s value field (name unknown): with `Name` `PropTemplate`
/// or `DebrisTemplate`, the name hash of a template.
const OSM_VALUE: u32 = 0x255f_12c0;

/// What each piece of a template's objects weighs once broken off (kg),
/// by id: the `_DebrisPhysics` `Mass` of the template its `OSMParameter`
/// `PropTemplate` names, the object its pieces become as its model's
/// collapse state `CreateObject`s them. Solano's villa gate
/// (`_pmcoutpost_gate`) names `prop_metal_lrg`, 40 kg: a leaf a car
/// throws aside, not the 19 t a stone block its size would weigh. From
/// the template database; that the leaves become such objects is read
/// from the state machine's verbs (not the executable).
pub fn piece_masses(db: &Layer<'_>, names: &HashMap<u32, &str>) -> HashMap<u32, f32> {
    let osm = Table::new(db, "OSMParameter");
    let debris = Table::new(db, "_DebrisPhysics");
    let (name, prop, mass) = (
        name_hash("Name"),
        name_hash("PropTemplate"),
        name_hash("Mass"),
    );
    let by_hash: HashMap<u32, u32> = names.iter().map(|(&id, n)| (name_hash(n), id)).collect();
    osm.templates()
        .filter_map(|id| {
            let named = osm
                .of_template(id)
                .iter()
                .find(|r| osm.word(r, name) == Some(prop))
                .and_then(|r| osm.word(r, OSM_VALUE))?;
            let t = *by_hash.get(&named)?;
            let kg = debris.float(debris.of_template(t).first()?, mass)?;
            (kg > 0.0).then_some((id, kg))
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

/// Each template's `PhysicsPropertyGravityScaler` `Scale`, by id: how
/// many times gravity pulls its objects (1.5 on the weapons and grenades
/// that lie to be taken: the `Assault Rifle`, the `Grenade`; PC data,
/// 2026-10-29; from samples).
pub fn gravity_scales(db: &Layer<'_>) -> HashMap<u32, f32> {
    let t = Table::new(db, "PhysicsPropertyGravityScaler");
    let scale = name_hash("Scale");
    t.templates()
        .filter_map(|id| {
            let s = t.float(t.of_template(id).first()?, scale)?;
            (s.is_finite() && s > 0.0).then_some((id, s))
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

/// How hard a vehicle of `mass` (kg) running into something at `speed`
/// (m/s) hits it, before the damage table scales it by what it hits (its
/// `ModifierKey` row) and the vehicle's kind of damage ([`RAM_KIND`]): a
/// tonne at a metre a second does 1, so a 2,000 kg Valiant at 10 m/s does
/// 20 and a 32,000 kg tank at 8 m/s 256. Invented: the data gives the kind
/// of a ram's damage and what each thing takes of it, not its size.
pub fn ram(mass: f32, speed: f32) -> f32 {
    mass * speed / 1000.0
}

/// How hard two bodies of `mass_a` and `mass_b` (kg) meet when they close
/// at `closing` (m/s, along the contact normal): a tenth of the closing
/// speed times the sum of the masses' square roots, so a 2,000 kg car
/// meeting a 6,000 kg truck at 2.6 m/s makes 31.8. Each side's factor
/// for the other (1 for vehicles) multiplies it; a body with no mass (a
/// fixed one) counts as [`FIXED_MASS`]. From the 360 code: the collision
/// event's size, 0x82163758 (masses and factors) and 0x82162BB8 (the
/// contacts' mean closing speed times it).
pub fn crash_size(mass_a: f32, mass_b: f32, closing: f32) -> f32 {
    let root = |m: f32| if m > 0.0 { m.sqrt() } else { FIXED_MASS.sqrt() };
    closing.abs() * (root(mass_a) + root(mass_b)) * 0.1
}

/// The mass a fixed body counts as in [`crash_size`]. From the 360 code
/// (0x82163758).
pub const FIXED_MASS: f32 = 15_000.0;

/// A crash smaller than this hurts neither side. From the 360 code
/// (0x8228B5F0).
pub const CRASH_FLOOR: f32 = 5.0;

/// What a crash of `size` ([`crash_size`]) does to one side, whose
/// `ModifierKey` row gives `cell` for the other side's `DamageKey`: a
/// hundredth of the size times the cell, and nothing unless that is
/// over 1. From the 360 code: 0x8228B5F0 (the floor and the hundredth),
/// 0x82287CF8 (each side's cell, and the 1).
pub fn crash_damage(size: f32, cell: f32) -> f32 {
    if size < CRASH_FLOOR {
        return 0.0;
    }
    let damage = size * 0.01 * cell;
    if damage > 1.0 { damage } else { 0.0 }
}

/// A ram's kind of damage where its vehicle's template names none: `Car`,
/// the damage table's column every vehicle template that names a
/// `DamageKey` of its own names (survey `runover`).
pub const RAM_KIND: &str = "Car";

/// What a vehicle of `mass` (kg) whose ram is of kind `key` (a name hash:
/// `Car`, `Tank`) does to a person whose `ModifierKey` row is `row`
/// (`Human`, `HumanArmor`, `Hero`), running into them at `speed` (m/s):
/// its [`ram`] times the damage table's cell. A `Human` takes three times a
/// `Car`'s and twice a `Tank`'s, a `HumanArmor` once either, the `Hero`
/// a fifth. A row or column the table lacks takes the `Human` row's, then
/// the ram itself. People take [`PERSON_RAM`] times the ram.
pub fn run_over(
    table: &crate::tuning::DamageTable,
    row: u32,
    key: u32,
    mass: f32,
    speed: f32,
) -> f32 {
    let scale = table
        .scale(row, key)
        .or_else(|| table.scale(name_hash("Human"), key))
        .unwrap_or(1.0);
    ram(mass, speed) * PERSON_RAM * scale
}

/// How much harder a ram hits a person than a prop. Invented (playtest
/// 6): at 1, a 1,000 kg car at 15 m/s did a `Human` 45 of their 100
/// health, and the playtest's soldiers got up from cars driven through
/// them (4 killed by vehicles in 25 minutes); at 5, one at 7 m/s
/// (25 km/h) kills.
pub const PERSON_RAM: f32 = 5.0;

/// A blast's reach and damage (`Explosive`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Explosion {
    /// Metres (`radius`).
    pub radius: f32,
    /// At its heart (`Damage`).
    pub damage: f32,
    /// At its edge (`minDamage`).
    pub least: f32,
    /// Its push at its heart (`MaxForce`; 500 where the template gives
    /// none, the schema's default, 0x823EF9F0).
    pub force: f32,
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
                    force: t.float(r, name_hash("MaxForce")).unwrap_or(500.0),
                    kind: keys.get(&id).copied().unwrap_or(0),
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A blast falls off with the distance to none at its edge, but never
    // under its least within reach (`fx_Explosion_HugeOil`'s 400, 95, 8 m).
    #[test]
    fn a_blast_falls_off_to_its_least_within_reach() {
        assert_eq!(blast_damage(400.0, 95.0, 8.0, 0.0), 400.0);
        assert_eq!(blast_damage(400.0, 95.0, 8.0, 4.0), 200.0);
        assert_eq!(blast_damage(400.0, 95.0, 8.0, 7.0), 95.0);
        assert_eq!(blast_damage(400.0, 95.0, 8.0, 8.5), 0.0);
        // A fuel tank burning from just under 465 by 50 a half second
        // burns out in 10 ticks.
        let b = BurnDown {
            below: 465.0,
            damage: 50.0,
            every: 0.5,
        };
        assert_eq!(b.seconds_to_burn_out(464.5), 5.0);
    }

    // A crash's size is a tenth of the closing speed times the masses'
    // roots summed; a fixed body counts as 15 t.
    #[test]
    fn a_crash_is_the_closing_speed_times_the_masses_roots() {
        let size = crash_size(2_000.0, 6_000.0, 2.6);
        assert!((size - 2.6 * (2_000f32.sqrt() + 6_000f32.sqrt()) * 0.1).abs() < 1e-4);
        assert_eq!(crash_size(2_000.0, 6_000.0, -2.6), size, "either way round");
        let fixed = crash_size(2_000.0, 0.0, 10.0);
        assert!((fixed - 10.0 * (2_000f32.sqrt() + 15_000f32.sqrt()) * 0.1).abs() < 1e-3);
    }

    // Each side takes a hundredth of the size times its cell, only once a
    // crash reaches 5 and only past 1 damage.
    #[test]
    fn a_crash_hurts_past_the_floor_and_past_one() {
        assert_eq!(crash_damage(4.9, 100.0), 0.0, "under the floor");
        assert_eq!(crash_damage(60.0, 1.0), 0.0, "0.6 is not past 1");
        assert!((crash_damage(60.0, 2.0) - 1.2).abs() < 1e-5);
        assert!((crash_damage(300.0, 2.0) - 6.0).abs() < 1e-4);
        assert_eq!(crash_damage(300.0, 0.0), 0.0, "a cell of nothing");
    }

    // A ram is a tonne a metre a second, times the table's cell for the
    // person's row and the vehicle's kind; a row the table lacks takes the
    // `Human` row's.
    #[test]
    fn a_run_over_is_the_ram_times_the_tables_cell() {
        let (human, hero, car) = (name_hash("Human"), name_hash("Hero"), name_hash(RAM_KIND));
        let mut data = vec![0u8; 0x48];
        let mut put = |v: u32| data.extend_from_slice(&v.to_le_bytes());
        put(2);
        put(1);
        put(human);
        put(hero);
        put(car);
        put(3.0f32.to_bits());
        put(0.2f32.to_bits());
        let table = crate::tuning::DamageTable::parse(&data).expect("table");
        assert_eq!(ram(2000.0, 10.0), 20.0);
        assert_eq!(run_over(&table, human, car, 2000.0, 10.0), 300.0);
        assert!((run_over(&table, hero, car, 2000.0, 10.0) - 20.0).abs() < 1e-5);
        assert_eq!(run_over(&table, 0x1234, car, 2000.0, 10.0), 300.0);
        assert_eq!(run_over(&table, human, 0x1234, 2000.0, 10.0), 100.0);
    }

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
