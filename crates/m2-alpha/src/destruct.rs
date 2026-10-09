//! The island's placed objects worn down by blasts until they come down,
//! as the template database and their models say: a template's `Health`
//! is how much it takes; its model's state machine says what it becomes
//! (pristine, collapsing with its pieces thrown, its lasting wreck).
//!
//! One whose model breaks apart ([`Breakup`](m2_engine::Breakup): a
//! building, a tower, a wall) is taken off the island and put up as a
//! prop, to come down as a mission's gates do ([`Props::wreck`]); one with
//! only a wreck is drawn as it. Props that come loose (fences, lamps,
//! signs) are knocked loose as before, not worn down here.
//!
//! Vehicles break what they run into as its template's
//! `PhysicsPropertyCrashable` says ([`Destruction::crash`]), the island's
//! objects and the state layers' props alike: plants and bushes flatten,
//! lampposts go over, fences break under a heavy enough vehicle; loose
//! props (crates) are shoved. What a vehicle breaks comes apart as the
//! vehicle knocks it on, not as a blast throws it: no building's rumble,
//! no smoke.

use std::collections::{HashMap, HashSet};

use m2_engine::durability::{Crash, CrashHow};
use m2_engine::{Install, StaticKey, World};
use m2_math::glam::{Mat4, Vec3};

use crate::combat::{Combat, Effect};
use crate::props::Props;

/// Keys the props put up for placed objects go under: entity ids of the
/// state layers stay well below.
const ADOPTED: u32 = 0xa000_0000;

/// The key placed object `i` is put up under as a prop.
pub fn key(i: usize) -> u32 {
    ADOPTED | i as u32
}

/// Whether a prop's key is one of the island's placed objects put up to
/// come down, rather than a state layer's entity.
pub fn adopted(key: u32) -> bool {
    key & ADOPTED == ADOPTED
}

/// A blast that wears things down: where, how far it reaches (metres),
/// and the damage at its heart, falling off to `least` at its edge
/// (`Explosive`'s `radius`, `Damage` and `minDamage`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    pub at: Vec3,
    pub radius: f32,
    pub damage: f32,
    pub least: f32,
}

impl Blast {
    /// The damage at `d` metres from its heart: none past its reach.
    pub fn at_distance(&self, d: f32) -> f32 {
        if d > self.radius {
            return 0.0;
        }
        let k = (d / self.radius.max(0.01)).clamp(0.0, 1.0);
        self.damage + (self.least - self.damage) * k
    }
}

/// A vehicle as it runs into things: where it is (its model's place),
/// its model's box, how fast it was going before the step just taken
/// stopped it against whatever it hit, and its mass (kg).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crasher {
    pub place: Mat4,
    pub bounds: (Vec3, Vec3),
    pub velocity: Vec3,
    pub mass: f32,
}

impl Crasher {
    /// Whether its box meets a model's box (`bounds`, model space) placed
    /// at `at`, for one its body hasn't touched (a bush with no
    /// collision): that box's middle comes within its reach of the
    /// vehicle's box, flat, and the two overlap in height (a helicopter
    /// or a vehicle on a bridge flattens nothing under it, as nothing
    /// over it).
    fn meets(&self, at: Mat4, (olo, ohi): (Vec3, Vec3)) -> bool {
        let (lo, hi) = self.bounds;
        // Its box's middle, and how far it reaches, in the vehicle's
        // space: it is hit if that comes within the vehicle's box.
        let centre = at.transform_point3((olo + ohi) * 0.5);
        let local = self.place.inverse().transform_point3(centre);
        let half = (ohi - olo) * 0.5;
        let reach = half.x.max(half.z).min(half.length() * 0.5);
        let nearest = local.clamp(lo, hi);
        let (low, high) = (centre.y - half.y, centre.y + half.y);
        let height = |y: f32| self.place.transform_point3(Vec3::new(0.0, y, 0.0)).y;
        (local - nearest).with_y(0.0).length() <= reach
            && low <= height(hi.y)
            && high >= height(lo.y)
    }

    /// What it does to what it runs into (under `key`), whose template
    /// has `crash` (if any), that comes `loose` or not: plants and bushes
    /// flatten under any vehicle, lampposts go over (knocked loose if they
    /// come loose), fences and the like break apart under one as heavy as
    /// they take, loose or not (round D10's tank shoved a whole fence
    /// section aside instead); a loose one with no such property (a
    /// crate) is shoved.
    fn does(&self, key: StaticKey, crash: Option<Crash>, loose: bool) -> Option<Crashed> {
        let down = crash.is_some_and(|c| match c.how {
            CrashHow::Flattened => true,
            CrashHow::KnockedOver => !loose,
            CrashHow::Breaks => self.mass >= c.takes,
        });
        if down {
            Some(Crashed::Down(key))
        } else if loose && crash.is_none_or(|c| self.mass >= c.takes) {
            Some(Crashed::Loose(key))
        } else {
            None
        }
    }
}

/// What [`Destruction::crash`] did to one of the island's placed objects
/// ([`StaticKey::Object`]) or a state layer's prop ([`StaticKey::Entity`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Crashed {
    /// Brought down (flattened, broken).
    Down(StaticKey),
    /// Comes loose: to be knocked loose (the vehicle's own rule).
    Loose(StaticKey),
}

/// What brought a placed object down, for how it comes down once its prop
/// has loaded.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cause {
    /// A blast that went off here: its pieces thrown from there, heard
    /// and seen coming down.
    Blast(Vec3),
    /// A vehicle of `mass` (kg) running into it at `velocity` (m/s): its
    /// pieces knocked on as the vehicle knocks loose what it hits.
    Crash { velocity: Vec3, mass: f32 },
}

/// The damage done to the island's placed objects, and those coming down.
#[derive(Debug, Default)]
pub struct Destruction {
    /// Damage taken so far, by placed object.
    taken: HashMap<usize, f32>,
    /// Destroyed, put up as props, waiting for their models to load to
    /// come down: (object, what did it).
    falling: Vec<(usize, Cause)>,
    /// Destroyed, by placed object.
    pub destroyed: HashSet<usize>,
}

impl Destruction {
    /// The damage placed object `i` has taken.
    pub fn damage(&self, i: usize) -> f32 {
        self.taken.get(&i).copied().unwrap_or(0.0)
    }

    /// Wears down what `blast` reaches; those it destroys come down.
    pub fn blast(&mut self, world: &mut World, props: &mut Props, install: &Install, blast: Blast) {
        for (i, d) in world.near(blast.at, blast.radius) {
            if self.destroyed.contains(&i)
                || world.destroyed.contains(&i)
                || world.loose_mass(i).is_some()
            {
                continue;
            }
            let o = world.index.objects[i];
            let Some(durability) = world.index.template_db.durability.get(&o.template) else {
                continue;
            };
            if durability.invincible || durability.health <= 0.0 {
                continue;
            }
            let Some(m) = world.object_model(i) else {
                continue;
            };
            if !m.has_wreck() {
                continue;
            }
            let hit = blast.at_distance(d);
            let taken = self.taken.entry(i).or_default();
            *taken += hit;
            if *taken < durability.health {
                continue;
            }
            eprintln!(
                "destruct: object {i} ({:08x}, template {:08x}) destroyed at {:.1}: {:.0} of {:.0} \
                 health, last {hit:.0} at {d:.1} m",
                o.model, o.template, o.position, taken, durability.health
            );
            self.bring_down(world, props, install, i, Cause::Blast(blast.at));
        }
    }

    /// Destroys placed object `i` by `cause`: put up as a prop to come
    /// down, else drawn as its wreck.
    fn bring_down(
        &mut self,
        world: &mut World,
        props: &mut Props,
        install: &Install,
        i: usize,
        cause: Cause,
    ) {
        self.destroyed.insert(i);
        let o = world.index.objects[i];
        if world.object_model(i).is_some_and(|m| m.has_wreck()) && props.adopt(install, key(i), &o)
        {
            self.falling.push((i, cause));
        } else {
            world.destroyed.insert(i);
        }
        world.take_static(i);
    }

    /// What `vehicle` runs into, the island's placed objects and the
    /// state layers' props alike, as each one's template's
    /// `PhysicsPropertyCrashable` has it ([`Crasher::does`]). Those
    /// brought down go at once; those that come loose are handed back for
    /// the vehicle to knock loose. What it runs into is what its body
    /// touched in the step just taken (`touched`: a long fence hit near
    /// its end post counts, as one hit in its middle does), and what its
    /// box meets ([`Crasher::meets`]: a bush with no collision).
    pub fn crash(
        &mut self,
        world: &mut World,
        props: &mut Props,
        install: &Install,
        vehicle: &Crasher,
        touched: &[StaticKey],
    ) -> Vec<Crashed> {
        let (lo, hi) = vehicle.bounds;
        let middle = vehicle.place.transform_point3((lo + hi) * 0.5);
        let radius = (hi - lo).length() * 0.5;
        let mut out = Vec::new();
        for (i, _) in world.near(middle, radius) {
            if self.destroyed.contains(&i) || world.destroyed.contains(&i) {
                continue;
            }
            let o = world.index.objects[i];
            let Some(m) = world.object_model(i) else {
                continue;
            };
            let key = StaticKey::Object(i);
            let at = Mat4::from_rotation_translation(o.rotation, o.position);
            if !touched.contains(&key) && !vehicle.meets(at, m.bounds) {
                continue;
            }
            let crash = world.index.template_db.crashable.get(&o.template).copied();
            let loose = world.loose_mass(i).is_some();
            match vehicle.does(key, crash, loose) {
                Some(Crashed::Down(_)) => {
                    eprintln!(
                        "destruct: object {i} ({:08x}) at {:.1} run into ({:?}) by a {:.0} kg \
                         vehicle at {:.1} m/s; {}",
                        o.model,
                        o.position,
                        crash.map(|c| c.how),
                        vehicle.mass,
                        vehicle.velocity.length(),
                        no_sound(world, o.template)
                    );
                    let cause = Cause::Crash {
                        velocity: vehicle.velocity,
                        mass: vehicle.mass,
                    };
                    self.bring_down(world, props, install, i, cause);
                    out.push(Crashed::Down(key));
                }
                Some(c) => out.push(c),
                None => {}
            }
        }
        for e in props.standing_near(middle, radius) {
            let (Some((at, bounds)), Some((p, _))) = (props.placed_bounds(e), props.get(e)) else {
                continue;
            };
            let (template, model, position) = (p.template, p.model, p.position);
            let key = StaticKey::Entity(e);
            if !touched.contains(&key) && !vehicle.meets(at, bounds) {
                continue;
            }
            let crash = world.index.template_db.crashable.get(&template).copied();
            let loose = props.loose_mass(&world.physics, e).is_some();
            match vehicle.does(key, crash, loose) {
                Some(Crashed::Down(_)) => {
                    eprintln!(
                        "destruct: prop {e:#x} ({model:08x}, of {}) at {position:.1} run into \
                         ({:?}) by a {:.0} kg vehicle at {:.1} m/s; {}",
                        props.layer(e).unwrap_or("a layer not noted"),
                        crash.map(|c| c.how),
                        vehicle.mass,
                        vehicle.velocity.length(),
                        no_sound(world, template)
                    );
                    props.wreck_crashed(&mut world.physics, e, |shifted| {
                        crate::game::flung(vehicle.velocity, vehicle.mass, shifted)
                    });
                    out.push(Crashed::Down(key));
                }
                Some(c) => out.push(c),
                None => {}
            }
        }
        out
    }

    /// Brings down those destroyed whose props have loaded: the island's
    /// copy goes, the prop breaks apart (or shows its wreck). One a blast
    /// destroyed is heard coming down, dust rising where its break-up
    /// names points; one a vehicle ran into is knocked on by it, and
    /// nothing is heard ([`no_sound`]).
    pub fn update(&mut self, world: &mut World, props: &mut Props, combat: &mut Combat) {
        self.falling.retain(|&(i, cause)| {
            let key = key(i);
            match props.get(key) {
                Some((_, true)) => {}
                Some((_, false)) => return true,
                // Taken down (should not be): drawn as its wreck instead.
                None => {
                    world.destroyed.insert(i);
                    return false;
                }
            }
            world.take(i);
            match cause {
                Cause::Blast(from) => {
                    props.wreck(&mut world.physics, key, Some(from));
                    if props.broke_apart(key) {
                        if let Some(at) = props.centre(key) {
                            combat.collapse(at);
                        }
                        for at in props.breakup_points(key) {
                            combat.show(Effect::Smoke { at, size: 4.0 }, 6.0);
                        }
                    }
                }
                Cause::Crash { velocity, mass } => {
                    props.wreck_crashed(&mut world.physics, key, |shifted| {
                        crate::game::flung(velocity, mass, shifted)
                    });
                }
            }
            false
        });
    }
}

/// Why nothing is heard as a vehicle breaks one of `template`'s: no field
/// read from the template database names a break sound yet (the building
/// rumble a blast's collapse plays is not one, and no gain is made up for
/// another). Names the template, and the bank its `SoundEffect` names if
/// it has one, for a survey to find the field by (`templates:<name>`,
/// `sounddb:<name>`).
fn no_sound(world: &World, template: u32) -> String {
    let db = &world.index.template_db;
    format!(
        "no break sound read for template {template:08x} {:?} (its SoundEffect's bank {}): none \
         played",
        db.names.get(&template).map_or("?", String::as_str),
        db.infos
            .get(&template)
            .and_then(|t| t.engine)
            .map_or_else(|| "none".to_owned(), |b| format!("{b:08x}"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A jeep: 4.4 m long along z, 2 m wide, 1.6 m high, standing at
    /// `at` facing +z.
    fn jeep(at: Vec3) -> Crasher {
        Crasher {
            place: Mat4::from_translation(at),
            bounds: (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.6, 2.2)),
            velocity: Vec3::new(0.0, 0.0, 8.0),
            mass: 1000.0,
        }
    }

    #[test]
    fn a_vehicle_meets_only_what_it_overlaps_in_height() {
        // A bush a metre across and high, just ahead of the jeep's nose.
        let bush = (Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5));
        let ahead = Mat4::from_translation(Vec3::new(0.0, 0.0, 2.5));
        assert!(jeep(Vec3::ZERO).meets(ahead, bush));
        // Off to its side, out of reach.
        let aside = Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0));
        assert!(!jeep(Vec3::ZERO).meets(aside, bush));
        // The jeep on a bridge 5 m over it, or flown over it: nothing.
        assert!(!jeep(Vec3::Y * 5.0).meets(ahead, bush));
        // A sign hung 5 m over the road: driven under.
        let sign = Mat4::from_translation(Vec3::new(0.0, 5.0, 2.5));
        assert!(!jeep(Vec3::ZERO).meets(sign, bush));
    }

    #[test]
    fn what_a_vehicle_does_follows_the_template() {
        let car = jeep(Vec3::ZERO);
        let key = StaticKey::Object(7);
        let crash = |how, takes| Some(Crash { how, takes });
        // A fence breaks under a vehicle as heavy as it takes, loose or
        // not; a lighter one leaves it standing.
        let fence = crash(CrashHow::Breaks, 950.0);
        assert_eq!(car.does(key, fence, false), Some(Crashed::Down(key)));
        assert_eq!(car.does(key, fence, true), Some(Crashed::Down(key)));
        let light = Crasher { mass: 900.0, ..car };
        assert_eq!(light.does(key, fence, false), None);
        // A lamppost that comes loose is knocked loose, else it goes over.
        let lamp = crash(CrashHow::KnockedOver, 0.0);
        assert_eq!(car.does(key, lamp, true), Some(Crashed::Loose(key)));
        assert_eq!(car.does(key, lamp, false), Some(Crashed::Down(key)));
        // A crate (no such property) is shoved; a rock stands.
        assert_eq!(car.does(key, None, true), Some(Crashed::Loose(key)));
        assert_eq!(car.does(key, None, false), None);
    }
}
