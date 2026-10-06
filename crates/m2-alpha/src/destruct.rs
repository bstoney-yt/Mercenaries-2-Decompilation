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

use std::collections::{HashMap, HashSet};

use m2_engine::{Install, World};
use m2_math::glam::Vec3;

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
            self.destroyed.insert(i);
            eprintln!(
                "destruct: object {i} ({:08x}, template {:08x}) destroyed at {:.1}: {:.0} of {:.0} \
                 health, last {hit:.0} at {d:.1} m",
                o.model, o.template, o.position, taken, durability.health
            );
            if props.adopt(install, key(i), &o) {
                self.falling.push((i, blast.at));
            } else {
                world.destroyed.insert(i);
            }
        }
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
