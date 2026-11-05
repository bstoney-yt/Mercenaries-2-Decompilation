//! A pursuit's gunships: the helicopters other than its transport that a
//! faction's pursuit sends after her, flown by the 360's AI pilot
//! ([`m2_engine::heli_pilot::HeliPilot::attack`]) and firing their guns
//! at her.
//!
//! From the 360 code:
//! - Each faction's pursuit table (0x82B7C6F8: the Allies' `114f350c`,
//!   the Chinese `b7e84db6`, the Guerrillas' `a9401062`, the UP's
//!   `fc3059d8`, the Pirates' `171b2ca7`, the Venezuelans' `4d5bf92a`;
//!   columns `When`, `SpawnType`, `Template`, `Threat1..3`) becomes, for
//!   each of her states and kinds of unit, one list a pursuit level: each
//!   row's template its count of times, in the table's order, then
//!   shuffled (0x82237968, 0x82235E70); a unit of that kind takes the
//!   next of its list, round and round (0x82235E08).
//! - An air unit (0x82239DA0, 0x8223ADD0) comes in 350 m from her about
//!   the way she faces (her heading + (0.5 - a random number) x 0.8
//!   rad), 70 m over the ground there (or 5 m over her, if higher),
//!   raised 5 m at a time (up to ten) until a box 15 m about it is clear,
//!   facing her and moving at her at 40 m/s.
//!
//! The alpha's: the shuffle's and the spawn's random numbers are its own
//! (the 360's from its generator, 0x8215E260); its gunships stay till
//! wrecked or the pursuit is over, then fly off (inferred: the 360 keeps
//! its units while its pursuit counts them).

use std::collections::{HashMap, HashSet};

use m2_assets::animation_table::AnimationTable;
use m2_assets::name_hash;
use m2_engine::Physics;
use m2_engine::heli_pilot::{AttackTarget, Gun, HeliPilot, Sight};
use m2_engine::tuning::{FlightTuning, PilotTuning};
use m2_math::glam::{Quat, Vec3};

use crate::scene::ParkedVehicle;

/// The faction tables, by the alpha's faction abbreviations
/// ([`crate::npc::faction_of`]), in the 360's order.
pub const FACTION_TABLES: [(&str, u32); 6] = [
    ("All", 0x114f_350c),
    ("Chi", 0xb7e8_4db6),
    ("Gur", 0xa940_1062),
    ("Oil", 0xfc30_59d8),
    ("Pir", 0x171b_2ca7),
    ("Vza", 0x4d5b_f92a),
];

/// The entities the gunships are placed as (their own range).
pub const FIRST_ENTITY: u32 = 0xfd00_0000;

/// How far from her an air unit comes in, how high over the ground and
/// over her, the step it is raised by and the box kept clear about it,
/// and how fast it comes in, m and m/s (0x8223ADD0, 0x82239DA0).
const FROM: f32 = 350.0;
const OVER_GROUND: f32 = 70.0;
const OVER_HER: f32 = 5.0;
const RAISE: f32 = 5.0;
pub const CLEAR: f32 = 15.0;
const COMING: f32 = 40.0;
/// The 360 lifts the velocity it starts with by this (0x82239DA0).
const COMING_UP: f32 = 8.0;
/// Leaving, a gunship is gone this far from her, m (invented, as a
/// delivery's, [`crate::flights`]).
pub const VANISH: f32 = 900.0;

/// One faction's lists: by her state and kind of unit (their names'
/// hashes), a list a level.
#[derive(Debug, Default)]
pub struct Lists {
    lists: HashMap<(&'static str, u32, u32), [Vec<String>; 3]>,
    next: HashMap<(&'static str, u32, u32, usize), usize>,
}

impl Lists {
    /// The lists from the faction tables (`table(hash)`), each template
    /// named through `names` (a template name's hash to it), shuffled by
    /// `rand` (0..1).
    pub fn build(
        mut table: impl FnMut(u32) -> Option<AnimationTable>,
        names: &HashMap<u32, String>,
        mut rand: impl FnMut() -> f32,
    ) -> Self {
        let mut lists: HashMap<(&'static str, u32, u32), [Vec<String>; 3]> = HashMap::new();
        for (faction, hash) in FACTION_TABLES {
            let Some(t) = table(hash) else {
                continue;
            };
            let col = |n: &str| t.column(n);
            let (Some(when), Some(kind), Some(template)) =
                (col("When"), col("SpawnType"), col("Template"))
            else {
                continue;
            };
            let threat = [col("Threat1"), col("Threat2"), col("Threat3")];
            for r in 0..t.rows() {
                let (Some(w), Some(k), Some(name)) = (
                    t.cell(r, when),
                    t.cell(r, kind),
                    t.cell(r, template).and_then(|h| names.get(&h)),
                ) else {
                    continue;
                };
                let entry = lists.entry((faction, w, k)).or_default();
                for (level, c) in threat.iter().enumerate() {
                    let n = c.and_then(|c| t.cell(r, c)).unwrap_or(0).min(64);
                    for _ in 0..n {
                        entry[level].push(name.clone());
                    }
                }
            }
        }
        // Shuffled: a random key each, sorted by it (0x82235E70).
        for set in lists.values_mut() {
            for list in set.iter_mut() {
                let mut keyed: Vec<(f32, String)> = list.drain(..).map(|n| (rand(), n)).collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
                *list = keyed.into_iter().map(|(_, n)| n).collect();
            }
        }
        Self {
            lists,
            next: HashMap::new(),
        }
    }

    /// Whether any were built.
    pub fn is_empty(&self) -> bool {
        self.lists.is_empty()
    }

    /// The next template of `faction`'s list for her state `when` and
    /// kind `kind` (their table names) at `level` (0 to 2), round and
    /// round (0x82235E08).
    pub fn next(
        &mut self,
        faction: &'static str,
        when: &str,
        kind: &str,
        level: usize,
    ) -> Option<String> {
        let key = (faction, name_hash(when), name_hash(kind));
        let list = self.lists.get(&key)?.get(level.min(2))?;
        if list.is_empty() {
            return None;
        }
        let at = self.next.entry((faction, key.1, key.2, level)).or_insert(0);
        let name = list[*at % list.len()].clone();
        *at = (*at + 1) % list.len();
        Some(name)
    }
}

/// Where an air unit comes in for her at `her`, facing `heading`, `r` a
/// random number (0x8223ADD0): `ground` the ground's height at a point,
/// `clear` whether a box `CLEAR` m about a point is clear. `None` if no
/// clear place within ten raises.
pub fn coming_in(
    her: Vec3,
    heading: f32,
    r: f32,
    ground: impl Fn(Vec3) -> Option<f32>,
    clear: impl Fn(Vec3) -> bool,
) -> Option<Vec3> {
    let a = heading + (0.5 - r) * 0.8;
    let at = her + Vec3::new(a.sin(), 0.0, a.cos()) * FROM;
    let mut height = OVER_GROUND;
    let floor = ground(at).unwrap_or(her.y);
    for _ in 0..10 {
        let y = (her.y + OVER_HER).max(floor + height);
        let p = Vec3::new(at.x, y, at.z);
        if clear(p) {
            return Some(p);
        }
        height += RAISE;
    }
    None
}

/// A gunship out.
#[derive(Debug)]
struct Gunship {
    entity: u32,
    faction: &'static str,
    template: String,
    pilot: Option<HeliPilot>,
    /// Where it comes in, moving at her (until its body is put there).
    at: Vec3,
    velocity: Vec3,
    /// Leaving: the pursuit over.
    leaving: bool,
}

/// What a gunner aims by ([`Shot::lead`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lead {
    pub velocity: Vec3,
    pub watched: Vec3,
    pub speed: f32,
    pub gravity: f32,
}

/// A pilot's ask of its gun this frame (`HeliAttack`'s fire, 0x822B0578).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    /// The vehicle (in the scene's) and where it aims.
    pub vehicle: usize,
    pub at: Vec3,
    /// How far its target is.
    pub dist: f32,
    /// On an attack run (its mind's +0x8C timer running): its pauses are
    /// a fifth as long.
    pub run: bool,
    /// The seat whose gun fires (of [`crate::combat::armed_seats`]: 0 the
    /// driver's) and the gun's priority there.
    pub seat: usize,
    pub priority: usize,
    /// A gunner's lead and drop ([`m2_engine::gunner_aim`]), when it aims
    /// so: its target's velocity, what it has seen of it, its round's
    /// speed and gravity scale.
    pub lead: Option<Lead>,
}

/// The gunships out, and where to put each as it loads.
#[derive(Debug, Default)]
pub struct Gunships {
    out: Vec<Gunship>,
    next: u32,
    /// Shots their pilots ask of their guns this frame
    /// ([`crate::combat::Combat::pilot_shots`]).
    pub shots: Vec<Shot>,
    /// Sent so far, for the log's tally.
    pub sent: usize,
    gone: HashSet<u32>,
    rng: u32,
}

impl Gunships {
    /// The alpha's random number for a spawn, 0..1.
    pub fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Whether the scene's vehicle placed for `entity` is a gunship.
    pub fn owns(&self, entity: u32) -> bool {
        self.out.iter().any(|g| g.entity == entity)
    }

    /// How many `faction` has out.
    pub fn out(&self, faction: &str) -> usize {
        self.out
            .iter()
            .filter(|g| g.faction == faction && !g.leaving)
            .count()
    }

    /// Sends one: `template` of `faction`, coming in at `at` moving at
    /// her; returns the entity it is to be placed as (the caller asks the
    /// scene for it).
    pub fn send(&mut self, faction: &'static str, template: String, at: Vec3, her: Vec3) -> u32 {
        let entity = FIRST_ENTITY + self.next;
        self.next += 1;
        let toward = (her - at).normalize_or_zero();
        self.out.push(Gunship {
            entity,
            faction,
            template,
            pilot: None,
            at,
            velocity: toward * COMING + Vec3::Y * COMING_UP,
            leaving: false,
        });
        self.sent += 1;
        entity
    }

    /// The pursuit of `faction` is over (or there is none): its gunships
    /// fly off.
    pub fn call_off(&mut self, faction: Option<&str>) {
        for g in &mut self.out {
            if faction.is_none_or(|f| f != g.faction) {
                g.leaving = true;
            }
        }
    }

    /// Flies the gunships for a step of `dt`: each placed in the air
    /// where it comes in as soon as its vehicle has loaded, its pilot then
    /// attacking her (at `her`, moving `her_velocity`) and asking its gun
    /// to fire ([`shots`](Self::shots)); leaving, it flies on 1000 m the
    /// way it heads and is taken out of the scene's vehicles' way past
    /// `vanish` metres from her. Returns the scene vehicles to drop.
    #[allow(clippy::too_many_arguments)]
    pub fn fly(
        &mut self,
        physics: &mut Physics,
        vehicles: &mut [ParkedVehicle],
        her: Vec3,
        her_velocity: Vec3,
        vanish: f32,
        mut rand: impl FnMut() -> f32,
        dt: f32,
    ) -> Vec<usize> {
        self.shots.clear();
        let mut drop = Vec::new();
        for g in &mut self.out {
            let Some(i) = vehicles
                .iter()
                .position(|v| v.spot.is_some_and(|(e, _, _)| e == g.entity))
            else {
                continue;
            };
            let v = &mut vehicles[i];
            if v.wrecked {
                self.gone.insert(g.entity);
                continue;
            }
            let id = match v.id {
                Some(id) => id,
                None => {
                    let toward = (her - g.at).with_y(0.0);
                    let yaw = toward.x.atan2(toward.z);
                    let id = physics.add_vehicle_turned(&v.spec, g.at, Quat::from_rotation_y(yaw));
                    physics.set_ai_pilot(id, true);
                    physics.start_rotor(id);
                    physics.set_vehicle_velocity(id, g.velocity);
                    v.id = Some(id);
                    let flight = v
                        .spec
                        .tuning
                        .flight_ai
                        .or(v.spec.tuning.flight)
                        .unwrap_or(FlightTuning::DEFAULT);
                    let tuning = v.spec.tuning.pilot.unwrap_or(PilotTuning::DEFAULT);
                    g.pilot = Some(HeliPilot::new(tuning, &flight));
                    eprintln!(
                        "gunships: {}'s {} in at {:.0}, {:.0} m from her",
                        g.faction,
                        g.template,
                        g.at,
                        g.at.distance(her)
                    );
                    id
                }
            };
            let Some(pilot) = g.pilot.as_mut() else {
                continue;
            };
            let Some(m) = physics.vehicle_transform(id) else {
                continue;
            };
            let (_, turn, at) = m.to_scale_rotation_translation();
            if g.leaving && at.distance(her) > vanish {
                drop.push(i);
                continue;
            }
            let velocity = physics.vehicle_velocity(id);
            let ground = physics.ground_under(id, at);
            pilot.airborne = physics.helicopter_airborne(id).unwrap_or(true);
            pilot.sense(at, turn, velocity, ground, dt);
            let size = {
                let (lo, hi) = v.spec.hull;
                (hi.y - lo.y, (hi.x - lo.x).max(hi.z - lo.z))
            };
            let round_speed = v
                .spec
                .armed
                .iter()
                .find(|m| m.priority == 0)
                .map(|m| m.weapon.ammo.velocity)
                .filter(|s| *s > 0.0);
            let stick = {
                let mut sight = Sight { physics, own: id };
                if g.leaving {
                    let ahead = Vec3::new(pilot.heading.sin(), 0.0, pilot.heading.cos());
                    pilot.fly_round_to(at + ahead * 1000.0, 1.0, true, true, size, &mut sight, dt)
                } else {
                    let target = AttackTarget {
                        at: her,
                        velocity: her_velocity,
                        unseen: 0.0,
                        round_speed,
                    };
                    let (stick, gun) = pilot.attack(&target, size, &mut sight, rand(), dt);
                    if let Gun::Fire(p) = gun {
                        self.shots.push(Shot {
                            vehicle: i,
                            at: p,
                            dist: (her - pilot.at).length(),
                            run: pilot.run_fire > 0.0,
                            seat: 0,
                            priority: 0,
                            lead: None,
                        });
                    }
                    stick
                }
            };
            physics.pilot_helicopter(id, stick);
        }
        let gone = &self.gone;
        self.out.retain(|g| !gone.contains(&g.entity));
        let dropped: HashSet<u32> = drop
            .iter()
            .filter_map(|&i| vehicles[i].spot.map(|(e, _, _)| e))
            .collect();
        self.out.retain(|g| !dropped.contains(&g.entity));
        drop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A list is taken round and round (0x82235E08), by level.
    #[test]
    fn a_list_is_taken_round_and_round() {
        let mut l = Lists::default();
        l.lists.insert(
            ("Vza", name_hash("OffRoad"), name_hash("Helicopter")),
            [vec!["A".into()], vec!["B".into(), "C".into()], vec![]],
        );
        let mut next = || l.next("Vza", "OffRoad", "Helicopter", 1);
        assert_eq!(
            [next(), next(), next()],
            [Some("B".into()), Some("C".into()), Some("B".into())]
        );
        assert_eq!(l.next("Vza", "OffRoad", "Helicopter", 2), None);
        assert_eq!(l.next("Vza", "OffRoad", "Helicopter", 0), Some("A".into()));
    }

    // An air unit comes in 350 m off about the way she faces, 70 m over
    // the ground there, raised till clear.
    #[test]
    fn it_comes_in_ahead_of_her_70_m_up() {
        let her = Vec3::new(0.0, 10.0, 0.0);
        let p = coming_in(her, 0.0, 0.5, |_| Some(0.0), |_| true).unwrap();
        assert!((p - Vec3::new(0.0, 70.0, 350.0)).length() < 1e-3, "{p}");
        let p = coming_in(her, 0.0, 0.5, |_| Some(0.0), |p| p.y > 80.0).unwrap();
        assert!((p.y - 85.0).abs() < 1e-3, "{p}");
        assert!(coming_in(her, 0.0, 0.5, |_| Some(0.0), |_| false).is_none());
    }
}
