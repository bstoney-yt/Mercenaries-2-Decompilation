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
//!
//! Only what has a building's collapse (its template's
//! `BuildingCollapseAnim`) is heard coming down as a building, with the
//! rumble; anything else (a fence, a sign) is heard as its template's own
//! `SoundKey` says (`_global_fencewoodpanel`'s `prop_sheetmetal_sml`), as
//! a blast brings it down or a vehicle does ([`break_sound`]).

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
    /// Its `DamageKey` (name hash; 0 for none, taken in full).
    pub kind: u32,
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

/// A ram's kind of damage: the vehicles' own `DamageKey` (`Car` on 520
/// vehicle templates, none on 259, nothing else; survey `runover`).
use m2_engine::durability::RAM_KIND;
/// A vehicle rams nothing slower than this, m/s (invented, as fast as it
/// must go to knock anything loose).
const RAM_SPEED: f32 = 1.0;

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

/// Whether a vehicle runs into what is under `key`: its body touched it,
/// else its box overlaps its collision (`boxed`), or, for what has no
/// collision (a bush), its drawn box (`drawn_meets`, [`Crasher::meets`]).
fn runs_into(
    world: &World,
    boxed: &[StaticKey],
    touched: &[StaticKey],
    key: StaticKey,
    drawn_meets: impl FnOnce() -> bool,
) -> bool {
    met(
        touched.contains(&key),
        world.physics.has_static(key).then(|| boxed.contains(&key)),
        drawn_meets,
    )
}

/// [`runs_into`]'s rule: `collision` is whether the box overlaps its
/// collision, `None` for what has none.
fn met(touched: bool, collision: Option<bool>, drawn_meets: impl FnOnce() -> bool) -> bool {
    touched || collision.unwrap_or_else(drawn_meets)
}

impl Crasher {
    /// Whether its box meets a model's box (`bounds`, model space) placed
    /// at `at`, for one its body hasn't touched (a bush with no
    /// collision, or something low its hull rides over): the two boxes,
    /// each turned as placed, overlap ([`boxes_meet`]: a long hedge hit
    /// at its end is met, a fence it drives along a few metres off is
    /// not), and so do their heights (a helicopter or a vehicle on a
    /// bridge flattens nothing under it, as nothing over it).
    fn meets(&self, at: Mat4, (olo, ohi): (Vec3, Vec3)) -> bool {
        let (lo, hi) = self.bounds;
        let centre = at.transform_point3((olo + ohi) * 0.5);
        let half = (ohi - olo) * 0.5;
        let (low, high) = (centre.y - half.y, centre.y + half.y);
        let height = |y: f32| self.place.transform_point3(Vec3::new(0.0, y, 0.0)).y;
        low <= height(hi.y)
            && high >= height(lo.y)
            && boxes_meet((self.place, self.bounds), (at, (olo, ohi)))
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

/// The parts of a model placed at `at` ([`m2_engine::ModelParts`]) that
/// `vehicle` breaks running into it, for a template whose
/// `PhysicsPropertyCrashable` records name nodes (`named`,
/// [`m2_engine::Templates::crash_parts`]): each standing part (not
/// `down`) whose intact box the vehicle's box meets, if its record says
/// the vehicle breaks it (a jeep flattens a hedge's section, breaks a
/// gate's leaf); one its body `touched` but whose parts' boxes it meets
/// none of, its nearest part (invented: the footage shows the parts hit
/// broken, not how the game picks them). `None` if no record names one of
/// the model's parts.
fn parts_hit(
    vehicle: &Crasher,
    at: Mat4,
    parts: &m2_engine::ModelParts,
    named: &[(u32, Crash)],
    down: impl Fn(usize) -> bool,
    touched: bool,
) -> Option<Vec<usize>> {
    let mine: Vec<(usize, Crash)> = named
        .iter()
        .filter_map(|&(node, crash)| Some((parts.part_named(node)?, crash)))
        .collect();
    if mine.is_empty() {
        return None;
    }
    let standing: Vec<(usize, Crash, (Vec3, Vec3))> = mine
        .iter()
        .filter(|(k, _)| !down(*k))
        .filter_map(|&(k, c)| Some((k, c, parts.part_box(k)?)))
        .collect();
    let mut met: Vec<(usize, Crash)> = standing
        .iter()
        .filter(|(_, _, b)| vehicle.meets(at, *b))
        .map(|&(k, c, _)| (k, c))
        .collect();
    if met.is_empty() && touched {
        let (lo, hi) = vehicle.bounds;
        let middle = vehicle.place.transform_point3((lo + hi) * 0.5);
        let gap = |(lo, hi): (Vec3, Vec3)| at.transform_point3((lo + hi) * 0.5).distance(middle);
        met.extend(
            standing
                .iter()
                .min_by(|a, b| gap(a.2).total_cmp(&gap(b.2)))
                .map(|&(k, c, _)| (k, c)),
        );
    }
    let key = StaticKey::Entity(0);
    let mut out: Vec<usize> = met
        .into_iter()
        .filter(|&(_, c)| vehicle.does(key, Some(c), false) == Some(Crashed::Down(key)))
        .map(|(k, _)| k)
        .collect();
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// Whether two boxes (each its model-space corners, placed by its
/// matrix) overlap, turned as they are: no line separates them, tried
/// along each box's three edges and every pair of one's edge with the
/// other's (the separating-axis test for two boxes).
fn boxes_meet(
    (a, (alo, ahi)): (Mat4, (Vec3, Vec3)),
    (b, (blo, bhi)): (Mat4, (Vec3, Vec3)),
) -> bool {
    // Each box's middle, its edges' directions, and how far it reaches
    // along each (scaled as its matrix has it).
    let turned = |m: Mat4, lo: Vec3, hi: Vec3| {
        let half = (hi - lo) * 0.5;
        let axes = [m.x_axis, m.y_axis, m.z_axis].map(|c| c.truncate());
        let reach = Vec3::new(
            half.x * axes[0].length(),
            half.y * axes[1].length(),
            half.z * axes[2].length(),
        );
        let axes = axes.map(|v| v.normalize_or_zero());
        (m.transform_point3((lo + hi) * 0.5), axes, reach)
    };
    let (ac, aa, ar) = turned(a, alo, ahi);
    let (bc, ba, br) = turned(b, blo, bhi);
    let apart = bc - ac;
    let parted = |l: Vec3| {
        // Edges near parallel give no direction to try.
        if l.length_squared() < 1e-8 {
            return false;
        }
        let along = |axes: &[Vec3; 3], reach: Vec3| {
            (0..3).map(|i| axes[i].dot(l).abs() * reach[i]).sum::<f32>()
        };
        apart.dot(l).abs() > along(&aa, ar) + along(&ba, br)
    };
    let edges = aa.iter().chain(&ba).copied();
    let crossed = aa.iter().flat_map(|&u| ba.iter().map(move |&v| u.cross(v)));
    !edges.chain(crossed).any(parted)
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

/// How fast a prop broken by a blast with nothing to show broken goes
/// flying away from it, and up, m/s (invented).
const FLY_AWAY: (f32, f32) = (5.0, 4.0);

impl Cause {
    /// How what it brought down of `mass` (kg), standing at `at`, flies
    /// when it has no wreck to show but comes loose: on with the vehicle
    /// ([`crate::game::flung`]), or away from the blast and up.
    fn flings(&self, at: Vec3, mass: f32) -> (Vec3, Vec3) {
        match *self {
            Cause::Crash { velocity, mass: by } => crate::game::flung(velocity, by, mass),
            Cause::Blast(from) => blown(from, at),
        }
    }
}

/// How something at `at` flies from a blast at `from`: away and up
/// ([`FLY_AWAY`]), turning over the way it goes.
pub fn blown(from: Vec3, at: Vec3) -> (Vec3, Vec3) {
    let away = (at - from).with_y(0.0).normalize_or(Vec3::X);
    let (out, up) = FLY_AWAY;
    (away * out + Vec3::Y * up, Vec3::Y.cross(away) * 3.0)
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
    /// What a vehicle's body touched and left standing, logged once each
    /// ([`held`]).
    held: HashSet<StaticKey>,
    /// Breaks to be heard at the next [`update`](Self::update): where, and
    /// the template's `SoundKey`.
    heard: Vec<(Vec3, u32)>,
    /// Damage the physics props have taken so far ([`wear`](Self::wear)).
    worn: HashMap<StaticKey, f32>,
    /// Worn down and blowing up, to be set off by the fight
    /// ([`crate::combat::Combat::set_off`]): where, and how big.
    pub set_off: Vec<(Vec3, f32)>,
    /// The island's objects destroyed whose links were followed
    /// ([`fall_linked`](Self::fall_linked)).
    linked: HashSet<usize>,
    /// The island's objects a vehicle broke some parts of, put up as
    /// props, waiting for their models to break those parts: (object,
    /// parts, the vehicle's velocity and mass).
    parting: Vec<(usize, Vec<usize>, Vec3, f32)>,
}

/// Whether template `t`'s objects blow up when destroyed: what it leaves
/// on dying (`SpawnOnDeath`) is a blast (`Explosive`).
pub fn explodes(db: &m2_engine::Templates, t: u32) -> bool {
    db.leavings
        .spawns
        .get(&t)
        .is_some_and(|s| s.iter().any(|s| db.explosions.contains_key(s)))
}

impl Destruction {
    /// The damage placed object `i` has taken.
    pub fn damage(&self, i: usize) -> f32 {
        self.taken.get(&i).copied().unwrap_or(0.0)
    }

    /// Wears down what `key` is (the island's placed object or a state
    /// layer's) by `damage` of kind `kind` (a `DamageKey`'s name hash; 0
    /// for none) landing at `at`: as much as its template's `ModifierKey`
    /// row of the damage table takes of that kind (a `PropFragile` chair
    /// all of a `Bullet`, half of an `Explosion`; a `Prop` box and a
    /// `Building` wall nothing of either), or `unrowed` of it for a
    /// template with no row. At its `Health` it breaks: its wreck if its
    /// model has one, else it goes flying ([`blown`]). Only what has
    /// something to show broken (a wreck, or a model that comes loose)
    /// is worn, and not what the scripts or the pickups bring down (a
    /// mission's gates, crates). True if it broke.
    pub fn wear(
        &mut self,
        world: &mut World,
        props: &mut Props,
        install: Option<&Install>,
        key: StaticKey,
        hit: (f32, u32, f32),
        at: Vec3,
    ) -> bool {
        self.wear_by(world, props, install, key, hit, Cause::Blast(at))
    }

    /// [`wear`](Self::wear) by `cause`: what breaks with nothing to show
    /// broken flies as it says (on with a vehicle that ran into it, away
    /// from a blast).
    fn wear_by(
        &mut self,
        world: &mut World,
        props: &mut Props,
        install: Option<&Install>,
        key: StaticKey,
        (damage, kind, unrowed): (f32, u32, f32),
        cause: Cause,
    ) -> bool {
        let at = match cause {
            Cause::Blast(at) => at,
            Cause::Crash { .. } => match key {
                StaticKey::Object(i) => world
                    .index
                    .objects
                    .get(i)
                    .map_or(Vec3::ZERO, |o| o.position),
                StaticKey::Entity(e) => props.centre(e).unwrap_or(Vec3::ZERO),
                _ => Vec3::ZERO,
            },
        };
        let template = match key {
            StaticKey::Object(i) => {
                if self.destroyed.contains(&i)
                    || world.destroyed.contains(&i)
                    || world.is_taken(i)
                    || world.knocked_loose(i).is_some()
                    || explodes(&world.index.template_db, world.index.objects[i].template)
                    || !world
                        .object_model(i)
                        .is_some_and(|m| m.has_wreck() || world.loose_mass(i).is_some())
                {
                    return false;
                }
                world.index.objects[i].template
            }
            StaticKey::Entity(e) => match props.get(e) {
                Some((p, _))
                    if !p.wrecked
                        && !adopted(e)
                        && !props.sturdy.contains(&e)
                        && props.knocked_loose(e).is_none() =>
                {
                    p.template
                }
                _ => return false,
            },
            _ => return false,
        };
        let db = &world.index.template_db;
        if crate::placed::kind(db, template).is_some()
            || db.names.get(&template).is_some_and(|n| n.contains("Cash"))
        {
            return false;
        }
        let Some(durability) = db.durability.get(&template).filter(|d| !d.invincible) else {
            return false;
        };
        let health = durability.health;
        if health <= 0.0 {
            return false;
        }
        let scale = match (db.modifiers.get(&template), db.damage.as_ref()) {
            _ if kind == 0 => 1.0,
            (Some(&row), Some(table)) => table.scale(row, kind).unwrap_or(0.0),
            _ => unrowed,
        };
        let hit = damage * scale;
        // A crash counts only past 1 once the cell has scaled it (the 360's
        // 0x82287CF8, as [`m2_engine::durability::crash_damage`]).
        let least = match cause {
            Cause::Crash { .. } => 1.0,
            Cause::Blast(_) => 0.0,
        };
        if hit <= least {
            return false;
        }
        let taken = match key {
            StaticKey::Object(i) => self.taken.entry(i).or_default(),
            _ => self.worn.entry(key).or_default(),
        };
        *taken += hit;
        if *taken < health {
            return false;
        }
        let taken = *taken;
        let name = db.names.get(&template).map_or("?", String::as_str);
        eprintln!(
            "destruct: {key:?} ({name}) broken at {at:.1}: {taken:.0} of {health:.0} health, last \
             {hit:.0} ({damage:.0} of kind {kind:08x} x {scale})"
        );
        let sound = db.sound_keys.get(&template).copied();
        // A state layer's prop whose template leaves a blast on dying
        // (`SpawnOnDeath` an `Explosive`: `_global_explosivebarrel`) blows
        // up where it stood, leaving nothing (the island's explosives go
        // off as they always have, by the fight's own count).
        if let StaticKey::Entity(e) = key
            && explodes(db, template)
        {
            let centre = props.centre(e).unwrap_or(at);
            let size = props
                .placed_bounds(e)
                .map_or(0.5, |(_, (lo, hi))| (hi - lo).length() * 0.5);
            eprintln!("destruct: {key:?} blows up at {centre:.1}");
            props.hide(&mut world.physics, [e]);
            self.set_off.push((centre, size));
            return true;
        }
        match key {
            StaticKey::Object(i) => match install {
                Some(install) => self.bring_down(world, props, install, i, cause),
                None => return false,
            },
            StaticKey::Entity(e) => {
                match cause {
                    Cause::Blast(at) => props.wreck(&mut world.physics, e, Some(at)),
                    Cause::Crash { velocity, mass } => {
                        props.wreck_crashed(&mut world.physics, e, |shifted| {
                            crate::game::flung(velocity, mass, shifted)
                        })
                    }
                };
                if let (Some(k), Some(c)) = (sound, props.centre(e)) {
                    self.heard.push((c, k));
                }
            }
            _ => {}
        }
        true
    }

    /// What `vehicle` does ramming what its body touched and its
    /// template says no vehicle breaks running into it, nor comes loose
    /// (a shack, a large jungle tree): it wears it down by the crash
    /// ([`m2_engine::durability::crash_size`] of its mass against a fixed
    /// body's, at its speed, and a hundredth of that), of the vehicles'
    /// kind, `Car`, as its row of the damage table takes of that (a shack
    /// twice it, a big tree's `Vegetation` ten times, a building or a
    /// canopy tree none), counting only past 1. From the 360 code: the
    /// same crash path as vehicle against vehicle (0x82162BB8,
    /// 0x82163758, 0x8228B5F0, 0x82287CF8; props and fixed bodies take
    /// part with a factor of 1, as vehicles). Inferred: its whole speed
    /// as the closing speed (the contact normal isn't kept here). Only
    /// going at [`RAM_SPEED`] or faster. True if it broke.
    fn rammed(
        &mut self,
        world: &mut World,
        props: &mut Props,
        install: &Install,
        key: StaticKey,
        vehicle: &Crasher,
    ) -> bool {
        let speed = vehicle.velocity.length();
        if speed < RAM_SPEED {
            return false;
        }
        let cause = Cause::Crash {
            velocity: vehicle.velocity,
            mass: vehicle.mass,
        };
        let size = m2_engine::durability::crash_size(vehicle.mass, 0.0, speed);
        let hit = (
            m2_engine::durability::crash_base(size),
            m2_assets::name_hash(RAM_KIND),
            0.0,
        );
        self.wear_by(world, props, Some(install), key, hit, cause)
    }

    /// Wears down what `blast` reaches ([`wear`](Self::wear)), the
    /// island's placed objects and the state layers' props, by its damage
    /// where it reaches each (as far as its box's middle less half its
    /// size), of its kind; those it breaks come down.
    pub fn blast(
        &mut self,
        world: &mut World,
        props: &mut Props,
        install: Option<&Install>,
        blast: Blast,
    ) {
        let mut reached: Vec<(StaticKey, f32)> = world
            .near(blast.at, blast.radius)
            .into_iter()
            .map(|(i, d)| (StaticKey::Object(i), d))
            .collect();
        for e in props.standing_near(blast.at, blast.radius) {
            if let Some((at, (lo, hi))) = props.placed_bounds(e) {
                let centre = at.transform_point3((lo + hi) * 0.5);
                let d = (centre.distance(blast.at) - (hi - lo).length() * 0.5).max(0.0);
                reached.push((StaticKey::Entity(e), d));
            }
        }
        for (key, d) in reached {
            let damage = blast.at_distance(d);
            self.wear(
                world,
                props,
                install,
                key,
                (damage, blast.kind, 1.0),
                blast.at,
            );
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
        let loose = world.loose_mass(i);
        if world.object_model(i).is_some_and(|m| m.has_wreck()) && props.adopt(install, key(i), &o)
        {
            self.falling.push((i, cause));
        } else if let Some(m) = loose {
            // No wreck to show, but it comes loose (a café chair, a
            // planter): it goes flying, as what is knocked loose does,
            // rather than vanish.
            let (velocity, spin) = cause.flings(o.position, m);
            world.knock(i, velocity, spin);
            if let Some(&k) = world.index.template_db.sound_keys.get(&o.template) {
                self.heard.push((o.position, k));
            }
        } else {
            // Drawn as its wreck at once: heard now, as one coming down
            // would be ([`update`](Self::update)), if it is no building.
            world.destroyed.insert(i);
            let db = &world.index.template_db;
            let building = db
                .durability
                .get(&o.template)
                .is_some_and(|d| d.collapse.is_some());
            if let Some(&k) = db.sound_keys.get(&o.template).filter(|_| !building) {
                self.heard.push((o.position, k));
            }
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
    /// box meets ([`Crasher::meets`]: a bush with no collision). What its
    /// body touched and still stands is logged once ([`held`]).
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
        // The collision its box overlaps: for what has collision (a
        // tree's trunk), that, not its drawn box (a tree's canopy reaches
        // over the road: playtest 4).
        let boxed = world.physics.statics_in_box(vehicle.place, vehicle.bounds);
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
            if !runs_into(world, &boxed, touched, key, || vehicle.meets(at, m.bounds)) {
                continue;
            }
            // Broken a part at a time: put up as a prop, its parts break
            // as it loads; nothing more while it does.
            if self.parting.iter().any(|p| p.0 == i) {
                continue;
            }
            if let Some(hit) = self.crash_object_parts(world, props, install, i, vehicle, touched) {
                out.extend(hit);
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
                        break_sound(world, o.template)
                    );
                    let cause = Cause::Crash {
                        velocity: vehicle.velocity,
                        mass: vehicle.mass,
                    };
                    self.bring_down(world, props, install, i, cause);
                    out.push(Crashed::Down(key));
                }
                Some(c) => out.push(c),
                None if touched.contains(&key)
                    && self.rammed(world, props, install, key, vehicle) =>
                {
                    out.push(Crashed::Down(key));
                }
                None if touched.contains(&key) && self.held.insert(key) => {
                    eprintln!("{}", held(world, key, o.model, o.template, crash, vehicle));
                }
                None => {}
            }
        }
        for e in props.standing_near(middle, radius) {
            let (Some((at, bounds)), Some((p, _))) = (props.placed_bounds(e), props.get(e)) else {
                continue;
            };
            let (template, model, position) = (p.template, p.model, p.position);
            let key = StaticKey::Entity(e);
            if !runs_into(world, &boxed, touched, key, || vehicle.meets(at, bounds)) {
                continue;
            }
            // What breaks a part at a time (a hedge's sections, a gate's
            // leaves): the parts it runs into, each as its own record says.
            if let Some(hit) = self.crash_parts(world, props, e, at, vehicle, touched) {
                out.extend(hit);
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
                        break_sound(world, template)
                    );
                    if let Some(&k) = world.index.template_db.sound_keys.get(&template) {
                        self.heard.push((position, k));
                    }
                    props.wreck_crashed(&mut world.physics, e, |shifted| {
                        crate::game::flung(vehicle.velocity, vehicle.mass, shifted)
                    });
                    out.push(Crashed::Down(key));
                }
                Some(c) => out.push(c),
                None if touched.contains(&key)
                    && self.rammed(world, props, install, key, vehicle) =>
                {
                    out.push(Crashed::Down(key));
                }
                None if touched.contains(&key) && self.held.insert(key) => {
                    eprintln!("{}", held(world, key, model, template, crash, vehicle));
                }
                None => {}
            }
        }
        out
    }

    /// What `vehicle` does to state layer prop `e` (placed at `at`) if its
    /// template breaks a part at a time ([`parts_hit`]): the parts it runs
    /// into break, the rest standing. `None` for any other prop (left to
    /// the whole-prop rule), else what it brought down (nothing, or the
    /// prop).
    fn crash_parts(
        &mut self,
        world: &mut World,
        props: &mut Props,
        e: u32,
        at: Mat4,
        vehicle: &Crasher,
        touched: &[StaticKey],
    ) -> Option<Vec<Crashed>> {
        let template = props.get(e)?.0.template;
        let named = world.index.template_db.crash_parts.get(&template)?.clone();
        let parts = props.model_parts(e)?.clone();
        let key = StaticKey::Entity(e);
        let hit = parts_hit(
            vehicle,
            at,
            &parts,
            &named,
            |k| props.part_down(e, k),
            touched.contains(&key),
        )?;
        let mut out = Vec::new();
        for k in hit {
            let position = props.get(e).map_or(Vec3::ZERO, |(p, _)| p.position);
            self.part_heard(world, template, &parts, at, k, position);
            eprintln!(
                "destruct: prop {e:#x} (of {}) part {k} of {} at {position:.1} run into by a {:.0} \
                 kg vehicle at {:.1} m/s; {}",
                props.layer(e).unwrap_or("a layer not noted"),
                parts.count(),
                vehicle.mass,
                vehicle.velocity.length(),
                break_sound(world, template)
            );
            props.wreck_part(&mut world.physics, e, k, |shifted| {
                crate::game::flung(vehicle.velocity, vehicle.mass, shifted)
            });
            if !out.contains(&Crashed::Down(key)) {
                out.push(Crashed::Down(key));
            }
        }
        Some(out)
    }

    /// The same for the island's placed object `i`: the
    /// parts it runs into break once the object is put up as a prop
    /// ([`update`](Self::update)), the island's copy and its collision
    /// going then. `None` for any other object, else what it brought
    /// down.
    fn crash_object_parts(
        &mut self,
        world: &World,
        props: &mut Props,
        install: &Install,
        i: usize,
        vehicle: &Crasher,
        touched: &[StaticKey],
    ) -> Option<Vec<Crashed>> {
        let o = world.index.objects[i];
        let at = Mat4::from_rotation_translation(o.rotation, o.position);
        let named = world.index.template_db.crash_parts.get(&o.template)?;
        let parts = world.object_model(i)?.parts.as_ref()?;
        let key = StaticKey::Object(i);
        let hit = parts_hit(vehicle, at, parts, named, |_| false, touched.contains(&key))?;
        if hit.is_empty() {
            return Some(Vec::new());
        }
        if !props.adopt(install, crate::destruct::key(i), &o) {
            return Some(Vec::new());
        }
        eprintln!(
            "destruct: object {i} ({:08x}) at {:.1}: parts {hit:?} of {} run into by a {:.0} kg \
             vehicle at {:.1} m/s; {}",
            o.model,
            o.position,
            parts.count(),
            vehicle.mass,
            vehicle.velocity.length(),
            break_sound(world, o.template)
        );
        for &k in &hit {
            self.part_heard(world, o.template, parts, at, k, o.position);
        }
        self.parting.push((i, hit, vehicle.velocity, vehicle.mass));
        Some(vec![Crashed::Down(key)])
    }

    /// A part breaking is heard where it stood, as its template's
    /// `SoundKey` says.
    fn part_heard(
        &mut self,
        world: &World,
        template: u32,
        parts: &m2_engine::ModelParts,
        at: Mat4,
        k: usize,
        or: Vec3,
    ) {
        if let Some(&s) = world.index.template_db.sound_keys.get(&template) {
            let c = parts
                .part_box(k)
                .map_or(or, |(lo, hi)| at.transform_point3((lo + hi) * 0.5));
            self.heard.push((c, s));
        }
    }

    /// Brings down what falls with what was brought down
    /// (`DestructionLink`, [`m2_engine::WorldIndex::falls_with`]): a state
    /// layer's prop wrecked takes those linked to it down (props wrecked
    /// where they stand; the island's objects as a blast there would), and
    /// so on down the links.
    fn fall_linked(&mut self, world: &mut World, props: &mut Props, install: Option<&Install>) {
        let mut fallen = props.take_fallen();
        // The island's objects destroyed since, by entity.
        fallen.extend(
            self.destroyed
                .iter()
                .filter(|i| !self.linked.contains(i))
                .filter_map(|&i| world.index.objects.get(i).map(|o| o.entity))
                .collect::<Vec<_>>(),
        );
        self.linked.extend(self.destroyed.iter().copied());
        let mut seen: HashSet<u32> = HashSet::new();
        while let Some(e) = fallen.pop() {
            if !seen.insert(e) {
                continue;
            }
            let Some(targets) = world.index.falls_with.get(&e).cloned() else {
                continue;
            };
            for t in targets {
                if props.get(t).is_some_and(|(p, _)| !p.wrecked) {
                    eprintln!("destruct: {t:#x} falls with {e:#x} (DestructionLink)");
                    let at = props.centre(t);
                    props.wreck(&mut world.physics, t, at);
                    fallen.extend(props.take_fallen());
                } else if let Some(i) = world.index.objects.iter().position(|o| o.entity == t)
                    && !self.destroyed.contains(&i)
                    && let Some(install) = install
                {
                    eprintln!("destruct: object {i} ({t:#x}) falls with {e:#x} (DestructionLink)");
                    let at = world.index.objects[i].position;
                    self.bring_down(world, props, install, i, Cause::Blast(at));
                    self.linked.insert(i);
                    fallen.push(t);
                }
            }
        }
    }

    /// Brings down those destroyed whose props have loaded: the island's
    /// copy goes, the prop breaks apart (or shows its wreck). One a blast
    /// destroyed is heard coming down, dust rising where its break-up
    /// names points, if it has a building's collapse; one a vehicle ran
    /// into is knocked on by it. Either, with no building's collapse, is
    /// heard as its template's `SoundKey` says ([`break_sound`]).
    pub fn update(
        &mut self,
        world: &mut World,
        props: &mut Props,
        combat: &mut Combat,
        install: Option<&Install>,
    ) {
        self.fall_linked(world, props, install);
        for (at, key) in self.heard.drain(..) {
            combat.broke(at, key);
        }
        // The island's objects broken a part at a time whose props have
        // loaded: the island's copy and its collision go, the prop stands
        // with those parts broken (and its others in the way).
        self.parting.retain(|(i, parts, velocity, mass)| {
            let key = key(*i);
            match props.get(key) {
                Some((_, true)) => {}
                Some((_, false)) => return true,
                None => return false,
            }
            world.take(*i);
            world.take_static(*i);
            for &k in parts.iter() {
                props.wreck_part(&mut world.physics, key, k, |shifted| {
                    crate::game::flung(*velocity, *mass, shifted)
                });
            }
            false
        });
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
            let template = world.index.objects[i].template;
            let db = &world.index.template_db;
            let building = db
                .durability
                .get(&template)
                .is_some_and(|d| d.collapse.is_some());
            let sound_key = db.sound_keys.get(&template).copied();
            match cause {
                Cause::Blast(from) => {
                    props.wreck(&mut world.physics, key, Some(from));
                    if props.broke_apart(key) && building {
                        if let Some(at) = props.centre(key) {
                            combat.collapse(at);
                        }
                        for at in props.breakup_points(key) {
                            combat.show(Effect::Smoke { at, size: 4.0 }, 6.0);
                            combat.show_fx(crate::combat::fx::COLLAPSE, at);
                        }
                    }
                }
                Cause::Crash { velocity, mass } => {
                    props.wreck_crashed(&mut world.physics, key, |shifted| {
                        crate::game::flung(velocity, mass, shifted)
                    });
                }
            }
            if !building && let Some(k) = sound_key {
                let at = props.centre(key).unwrap_or(world.index.objects[i].position);
                combat.broke(at, k);
            }
            false
        });
    }
}

/// What is heard as one of `template`'s breaks, for the log: a building
/// (it has `BuildingCollapseAnim`) comes down to a blast with the
/// collapse's rumble; anything else is heard as its `SoundKey` says
/// (`sound: key ... plays ...` logs what that is), or not at all if it
/// has none (no other sound stands in).
fn break_sound(world: &World, template: u32) -> String {
    let db = &world.index.template_db;
    let name = db.names.get(&template).map_or("?", String::as_str);
    let building = db
        .durability
        .get(&template)
        .is_some_and(|d| d.collapse.is_some());
    match db.sound_keys.get(&template) {
        _ if building => {
            format!("template {template:08x} {name:?} has a building's collapse: no break sound")
        }
        Some(k) => format!("template {template:08x} {name:?} heard by its SoundKey {k:08x}"),
        None => format!("template {template:08x} {name:?} has no SoundKey: none played"),
    }
}

/// The line logged for what a vehicle's body touched and left standing
/// (`key`, of `model` and `template`, whose template has `crash`, if
/// any): which template holds it, by name, and what its
/// `PhysicsPropertyCrashable` reads, for a run to tell a wall the data
/// keeps standing from one it should break.
fn held(
    world: &World,
    key: StaticKey,
    model: u32,
    template: u32,
    crash: Option<Crash>,
    vehicle: &Crasher,
) -> String {
    format!(
        "destruct: a {:.0} kg vehicle at {:.1} m/s held by {key:?} ({model:08x}), template \
         {template:08x} {:?}, crashable {crash:?}",
        vehicle.mass,
        vehicle.velocity.length(),
        world
            .index
            .template_db
            .names
            .get(&template)
            .map_or("?", String::as_str),
    )
}

#[cfg(test)]
mod tests {
    use m2_math::glam::Quat;

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
    fn a_tree_with_collision_goes_over_only_when_its_trunk_is_met() {
        // A jeep 2 m from a trunk, inside the tree's 8 m canopy box: the
        // canopy's box meets it, the trunk's collision doesn't.
        let tree = (Vec3::new(-4.0, 0.0, -4.0), Vec3::new(4.0, 10.0, 4.0));
        let at = Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0));
        let canopy = || jeep(Vec3::ZERO).meets(at, tree);
        assert!(canopy());
        assert!(!met(false, Some(false), canopy));
        assert!(met(false, Some(true), canopy));
        assert!(met(true, Some(false), canopy));
        // A bush with no collision: its drawn box decides.
        assert!(met(false, None, canopy));
        assert!(!met(false, None, || false));
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
    fn a_vehicle_meets_what_its_box_overlaps_not_what_is_near() {
        // A fence 20 m long along x, 2 m high, 0.2 m thick.
        let fence = (Vec3::new(-10.0, 0.0, -0.1), Vec3::new(10.0, 2.0, 0.1));
        let at = Mat4::IDENTITY;
        // The jeep driving along it, facing +x, its side 3 m and then
        // 4 m off: it passes the fence by.
        let along = |z: f32| Crasher {
            place: Mat4::from_rotation_translation(
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                Vec3::new(0.0, 0.0, z),
            ),
            ..jeep(Vec3::ZERO)
        };
        assert!(!along(4.0).meets(at, fence));
        assert!(!along(5.0).meets(at, fence));
        // A 4 m panel a metre beside it: passed by too.
        let panel = (Vec3::new(-2.0, 0.0, -0.1), Vec3::new(2.0, 2.0, 0.1));
        assert!(!along(2.0).meets(at, panel));
        // Scraping it: met.
        assert!(along(1.05).meets(at, fence));
        // A hedge 20 m long, its end run into by the jeep's nose: met,
        // though its middle is 9 m off.
        let hedge = (Vec3::new(-10.0, 0.0, -0.5), Vec3::new(10.0, 1.5, 0.5));
        assert!(jeep(Vec3::new(9.5, 0.0, -2.5)).meets(at, hedge));
        // Turned across the jeep's way at 45 degrees, its end in front of
        // the nose: met; the jeep 2.5 m further along it, past its end:
        // not.
        let turned = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let end = turned.transform_point3(Vec3::X * 9.5);
        assert!(jeep(end - Vec3::Z * 2.5).meets(turned, hedge));
        let past = turned.transform_point3(Vec3::X * 12.0);
        assert!(!jeep(past - Vec3::Z * 2.5).meets(turned, hedge));
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

    #[test]
    fn a_ram_into_something_fixed_is_the_360s_crash() {
        use m2_engine::durability::{crash_base, crash_size};
        // A jeep at 10 m/s, a tank at 8, against a fixed body (15 t).
        let jeep = crash_base(crash_size(1000.0, 0.0, 10.0));
        let tank = crash_base(crash_size(32_000.0, 0.0, 8.0));
        assert!((jeep - 1.541).abs() < 1e-3, "{jeep}");
        assert!((tank - 2.411).abs() < 1e-3, "{tank}");
        // Twice the speed, twice the crash; nothing at a crawl.
        let fast = crash_base(crash_size(32_000.0, 0.0, 16.0));
        assert!((fast - 2.0 * tank).abs() < 1e-3);
        assert_eq!(crash_base(crash_size(1000.0, 0.0, 0.3)), 0.0);
    }

    #[test]
    fn what_a_blast_breaks_with_no_wreck_flies_away_and_up() {
        let (velocity, spin) = blown(Vec3::ZERO, Vec3::new(3.0, 0.5, 0.0));
        assert!(velocity.x > 0.0 && velocity.y > 0.0 && velocity.z.abs() < 1e-5);
        assert!(spin.length() > 0.0);
        // Right on top of it: still away (along x) and up.
        let (velocity, _) = blown(Vec3::ONE, Vec3::ONE);
        assert!(velocity.x > 0.0 && velocity.y > 0.0);
    }

    #[test]
    fn what_leaves_a_blast_on_dying_blows_up() {
        let mut db = m2_engine::Templates::default();
        // A barrel that leaves an explosion; a crate that leaves a pickup.
        db.leavings.spawns.insert(1, vec![10]);
        db.leavings.spawns.insert(2, vec![20]);
        db.explosions.insert(
            10,
            m2_engine::durability::Explosion {
                radius: 6.0,
                damage: 300.0,
                least: 0.0,
                kind: 0,
            },
        );
        assert!(explodes(&db, 1));
        assert!(!explodes(&db, 2));
        assert!(!explodes(&db, 3));
    }
}
