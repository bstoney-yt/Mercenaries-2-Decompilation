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
//! `PhysicsPropertyCrashable` says ([`Destruction::crash`]): plants and
//! bushes flatten, lampposts go over, fences break under a heavy enough
//! vehicle; loose props (crates) are shoved.

use std::collections::{HashMap, HashSet};

use m2_engine::durability::CrashHow;
use m2_engine::{Install, World};
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

/// What [`Destruction::crash`] did to one placed object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Crashed {
    /// Brought down (flattened, broken).
    Down(usize),
    /// Comes loose: to be knocked loose (the vehicle's own rule).
    Loose(usize),
}

/// The damage done to the island's placed objects, and those coming down.
#[derive(Debug, Default)]
pub struct Destruction {
    /// Damage taken so far, by placed object.
    taken: HashMap<usize, f32>,
    /// Destroyed, put up as props, waiting for their models to load to
    /// come down: (object, the blast that did it).
    falling: Vec<(usize, Vec3)>,
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
            self.bring_down(world, props, install, i, blast.at);
        }
    }

    /// Destroys placed object `i`, as by something at `from`: put up as a
    /// prop to come down, else drawn as its wreck.
    fn bring_down(
        &mut self,
        world: &mut World,
        props: &mut Props,
        install: &Install,
        i: usize,
        from: Vec3,
    ) {
        self.destroyed.insert(i);
        let o = world.index.objects[i];
        if world.object_model(i).is_some_and(|m| m.has_wreck()) && props.adopt(install, key(i), &o)
        {
            self.falling.push((i, from));
        } else {
            world.destroyed.insert(i);
        }
        world.take_static(i);
    }

    /// What `vehicle` runs into, as each one's template's
    /// `PhysicsPropertyCrashable` has it: plants and bushes flatten under
    /// any vehicle, lampposts go over (knocked loose if they come loose),
    /// fences and the like break apart under one as heavy as they take,
    /// loose or not (round D10's tank shoved a whole fence section aside
    /// instead). Those
    /// brought down go at once; those that come loose are handed back,
    /// with every loose prop it runs into that has no such property (a
    /// crate), for the vehicle to knock loose. Only what its box overlaps.
    pub fn crash(
        &mut self,
        world: &mut World,
        props: &mut Props,
        install: &Install,
        vehicle: &Crasher,
    ) -> Vec<Crashed> {
        let (lo, hi) = vehicle.bounds;
        let middle = vehicle.place.transform_point3((lo + hi) * 0.5);
        let back = vehicle.place.inverse();
        let mut out = Vec::new();
        for (i, _) in world.near(middle, (hi - lo).length() * 0.5) {
            if self.destroyed.contains(&i) || world.destroyed.contains(&i) {
                continue;
            }
            let o = world.index.objects[i];
            let Some(m) = world.object_model(i) else {
                continue;
            };
            // Its box's middle, and how far it reaches, in the vehicle's
            // space: it is hit if that comes within the vehicle's box.
            let (olo, ohi) = m.bounds;
            let centre = o.position + o.rotation * ((olo + ohi) * 0.5);
            let local = back.transform_point3(centre);
            let half = (ohi - olo) * 0.5;
            let reach = half.x.max(half.z).min(half.length() * 0.5);
            let nearest = local.clamp(lo, hi);
            let low = centre.y - half.y;
            let top = vehicle.place.transform_point3(Vec3::new(0.0, hi.y, 0.0)).y;
            if (local - nearest).with_y(0.0).length() > reach || low > top {
                continue;
            }
            let loose = world.loose_mass(i).is_some();
            let crash = world.index.template_db.crashable.get(&o.template).copied();
            let down = match crash {
                Some(c) => match c.how {
                    CrashHow::Flattened => true,
                    CrashHow::KnockedOver => !loose,
                    CrashHow::Breaks => vehicle.mass >= c.takes,
                },
                None => false,
            };
            if down {
                eprintln!(
                    "destruct: object {i} ({:08x}, template {:08x}) at {:.1} run into ({:?}) by a \
                     {:.0} kg vehicle at {:.1} m/s",
                    o.model,
                    o.template,
                    o.position,
                    crash.map(|c| c.how),
                    vehicle.mass,
                    vehicle.velocity.length()
                );
                let from = vehicle.place.w_axis.truncate() - vehicle.velocity.with_y(0.0);
                self.bring_down(world, props, install, i, from);
                out.push(Crashed::Down(i));
            } else if loose && crash.is_none_or(|c| vehicle.mass >= c.takes) {
                out.push(Crashed::Loose(i));
            }
        }
        out
    }

    /// Brings down those destroyed whose props have loaded: the island's
    /// copy goes, the prop breaks apart (or shows its wreck), heard coming
    /// down, dust rising where its break-up names points.
    pub fn update(&mut self, world: &mut World, props: &mut Props, combat: &mut Combat) {
        self.falling.retain(|&(i, blast)| {
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
            props.wreck(&mut world.physics, key, Some(blast));
            if props.broke_apart(key) {
                if let Some(at) = props.centre(key) {
                    combat.collapse(at);
                }
                for at in props.breakup_points(key) {
                    combat.show(Effect::Smoke { at, size: 4.0 }, 6.0);
                }
            }
            false
        });
    }
}
