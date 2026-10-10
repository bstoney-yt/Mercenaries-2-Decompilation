//! The placed props a vehicle, gunfire or a blast should knock about or
//! break, and the check that tries each one (`--props-check`).
//!
//! What the data says of a placed object's template (PC survey
//! `breakables`, round 6, over VzaCon001's and the villa's ground):
//!
//! - `_PropPhysics` (102 templates) marks a loose physics prop: chairs,
//!   benches, café tables, planters, boxes, barrels, barricades, trash,
//!   dumpsters, containers, car ruins. `_BuildingPhysics` marks what
//!   stands fixed. From samples.
//! - `ModifierKey` names the damage table's row for what each kind of
//!   damage does to it, of the damage: `PropFragile` (chairs, planters,
//!   tables, barricades C and D) takes all of a `Bullet`, half of an
//!   `Explosion`, 100 times a `MeleeBash`; `Prop` (boxes, barrels, trash
//!   bags, barricades A and B) takes nothing of anything; `Fence` nothing
//!   of a `Bullet`, a quarter of an `Explosion`; `Car` 0.025 of a
//!   `Bullet`. From the data (the `ModifierKey` asset).
//! - `Health` is what it takes before it breaks; `PhysicsPropertyCrashable`
//!   how a vehicle running into it breaks it ([`m2_engine::durability`]).
//!
//! What each should do when hit ([`expected`]) follows from those; where
//! the data doesn't say (a vehicle's crash damage, whether rounds push a
//! prop), more than one reaction is taken as right.

use m2_engine::durability::{Crash, CrashHow, PropPhysics};
use m2_engine::{StaticKey, Templates};
use m2_math::glam::{Mat4, Quat, Vec3};

/// How a placed prop reacted to a hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Reaction {
    /// Nothing, and nothing of it in the way: a vehicle or a round goes
    /// through it.
    None,
    /// It stands, in the way.
    Block,
    /// Knocked loose whole: shoved, toppled, thrown.
    Move,
    /// Gone, with nothing left of it: no wreck, no pieces (its model
    /// has neither). To the player it is driven or shot through.
    Vanish,
    /// Broken: its wreck shown, or its pieces flying.
    Break,
}

/// What hit it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    Vehicle,
    Gun,
    Blast,
}

impl HitKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "vehicle" => Some(Self::Vehicle),
            "gun" => Some(Self::Gun),
            "blast" => Some(Self::Blast),
            _ => None,
        }
    }
}

/// What a template's objects are made of, as the data says, for
/// [`expected`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Makeup {
    pub physics: Option<PropPhysics>,
    pub crash: Option<Crash>,
    /// `Health`, where above 0 (car ruins have 0).
    pub health: Option<f32>,
    /// Of a `Bullet`'s and an `Explosion`'s damage, what it takes (its
    /// `ModifierKey` row; 0 without one).
    pub bullet: f32,
    pub explosion: f32,
    /// Of a crash's damage by a car (the table's `Car` column), what it
    /// takes: whether a car running into it wears it down at all (the
    /// crash's own damage the data doesn't give).
    pub crash_damage: f32,
}

impl Makeup {
    /// Template `t`'s.
    pub fn of(templates: &Templates, t: u32) -> Self {
        let row = templates.modifiers.get(&t).copied();
        let scale = |column: &str| {
            row.zip(templates.damage.as_ref())
                .and_then(|(r, d)| d.scale(r, m2_assets::name_hash(column)))
                .unwrap_or(0.0)
        };
        Self {
            physics: templates.prop_physics.get(&t).copied(),
            crash: templates.crashable.get(&t).copied(),
            health: templates.health.get(&t).copied().filter(|h| *h > 0.0),
            bullet: scale("Bullet"),
            explosion: scale("Explosion"),
            crash_damage: scale("Car"),
        }
    }

    /// Whether it is a prop the check tries: a loose physics prop, or one
    /// a vehicle breaks or knocks over (fences, lamps, signs). Plants and
    /// bushes that only flatten are the tank's round (7).
    pub fn is_prop(&self) -> bool {
        self.physics.is_some() || self.crash.is_some_and(|c| c.how != CrashHow::Flattened)
    }
}

/// The hits the check tries, as the game's own would land.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hits {
    /// The vehicle's mass (kg) and speed (m/s).
    pub vehicle: (f32, f32),
    /// Rounds fired, and each one's `Damage`.
    pub rounds: (u32, f32),
    /// The blast's damage where it reaches the prop.
    pub blast: f32,
}

/// What the data says `m` should do when hit by `hit` of `hits`: every
/// reaction taken as right.
pub fn expected(m: &Makeup, hit: HitKind, hits: &Hits) -> &'static [Reaction] {
    use Reaction::{Block, Break, Move};
    let heavy = |vehicle: f32| {
        m.physics
            .is_some_and(|p| p.heavy || p.mass.is_some_and(|k| k >= vehicle))
    };
    match hit {
        HitKind::Vehicle => {
            let (mass, _) = hits.vehicle;
            match m.crash.map(|c| (c.how, c.takes)) {
                Some((CrashHow::Breaks, takes)) if mass >= takes => &[Break],
                Some((CrashHow::Breaks, _)) => &[Block],
                Some((CrashHow::KnockedOver | CrashHow::Flattened, _)) => &[Break, Move],
                // One the table's `Car` column wears down may break (as
                // the crash's damage, which the data doesn't give, says).
                None if heavy(mass) && m.crash_damage > 0.0 && m.health.is_some() => {
                    &[Block, Move, Break]
                }
                None if heavy(mass) => &[Block, Move],
                None if m.crash_damage > 0.0 && m.health.is_some() => &[Break, Move],
                None => &[Move],
            }
        }
        HitKind::Gun => {
            let (n, damage) = hits.rounds;
            match m.health {
                Some(h) if m.bullet > 0.0 && n as f32 * damage * m.bullet >= h => &[Break],
                // Whether rounds push a prop, the data doesn't say.
                _ => &[Block, Move],
            }
        }
        HitKind::Blast => match m.health {
            Some(h) if m.explosion > 0.0 && hits.blast * m.explosion >= h => &[Break],
            _ if m.physics.is_some() && !heavy(f32::MAX) => &[Move, Break],
            _ if m.physics.is_some() => &[Block, Move],
            _ => &[Block],
        },
    }
}

/// The check's vehicle: a jeep's box (4.4 m long, 2 m wide, 1.6 m
/// high, invented) at `speed` (m/s) of `mass` (kg).
pub const JEEP_BOX: (Vec3, Vec3) = (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.6, 2.2));

/// The check's vehicle placed to run into what stands at `centre` (on
/// ground `floor`) along `forward`, its nose half a metre into it.
pub fn crasher_at(
    centre: Vec3,
    floor: f32,
    forward: Vec3,
    mass: f32,
    speed: f32,
) -> crate::destruct::Crasher {
    let yaw = forward.x.atan2(forward.z);
    let back = centre.with_y(floor) - forward * (JEEP_BOX.1.z - 0.5);
    crate::destruct::Crasher {
        place: Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), back),
        bounds: JEEP_BOX,
        velocity: forward * speed,
        mass,
    }
}

/// One prop the check tries: the island's placed object or a state
/// layer's entity, its template and where it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Item {
    key: StaticKey,
    template: u32,
    position: Vec3,
}

/// `--props-check` tries what stands within this of her, metres (as
/// `--placed-check`).
const CHECK_NEAR: f32 = 1500.0;
/// The props are tried a patch of ground at a time, this wide (metres),
/// with her put down in its middle so its models stream in.
const PATCH: f32 = 60.0;
/// How long a patch's models are waited for, seconds.
const PATCH_WAIT: f32 = 8.0;
/// How long a prop is left after its hit before it is looked at, seconds.
const SETTLE: f32 = 0.25;
/// The check's vehicle's speed, m/s (36 km/h; invented).
const SPEED: f32 = 10.0;
/// The blast goes off this far from the prop's box, metres (invented).
const BLAST_OFF: f32 = 1.0;
/// The rounds are fired from this far off, metres.
const SHOT_FROM: f32 = 8.0;

/// The name of a damage table row, for the log.
fn row_name(templates: &Templates, t: u32) -> &'static str {
    const ROWS: [&str; 14] = [
        "PropFragile",
        "Prop",
        "Fence",
        "Shack",
        "Building",
        "Car",
        "Tank",
        "Armor",
        "MediumArmor",
        "Vegetation",
        "Bunker",
        "Terrain",
        "Canopy",
        "Human",
    ];
    templates
        .modifiers
        .get(&t)
        .and_then(|&k| ROWS.into_iter().find(|r| m2_assets::name_hash(r) == k))
        .unwrap_or("-")
}

/// With `--props-check <vehicle|gun|blast>` (testing): every placed prop
/// in play within [`CHECK_NEAR`] of her ([`Makeup::is_prop`]; not the
/// pickups, crates and guns of [`crate::placed`], nor the explosives) is
/// hit by `hit` as the game's own would land ([`crate::game::Game::crash_with`],
/// [`crate::game::Game::hit_world_with`]) and looked at a moment later,
/// a patch of ground at a time; each logs a `props-check:` line with
/// what it did and what the data says it should ([`expected`]), then a
/// summary. She goes back where she stood.
pub fn check(
    game: &mut crate::game::Game,
    hit: HitKind,
    area: Option<[f32; 4]>,
    tick: f32,
) -> usize {
    use crate::player::Intent;
    let Some(start) = game.scene.player.as_ref().map(|p| p.character.position) else {
        return 0;
    };
    let hp = game.combat.hp;
    let Some(world) = game.scene.world.as_ref() else {
        return 0;
    };
    // In the area if one is given (x, z corners), else near her.
    let inside = |p: Vec3| match area {
        Some([x0, z0, x1, z1]) => p.x >= x0 && p.x <= x1 && p.z >= z0 && p.z <= z1,
        None => p.distance(start) < CHECK_NEAR,
    };
    let templates = &world.index.template_db;
    let explosive: std::collections::HashSet<usize> =
        game.scene.explosives.iter().map(|e| e.object).collect();
    // Not money (round 14's), nor what the pickups round tries.
    let wanted = |t: u32| {
        Makeup::of(templates, t).is_prop()
            && crate::placed::kind(templates, t).is_none()
            && !templates.names.get(&t).is_some_and(|n| n.contains("Cash"))
    };
    let mut items: Vec<Item> = Vec::new();
    for (i, o) in world.index.objects.iter().enumerate() {
        if inside(o.position) && !explosive.contains(&i) && !world.is_taken(i) && wanted(o.template)
        {
            items.push(Item {
                key: StaticKey::Object(i),
                template: o.template,
                position: o.position,
            });
        }
    }
    for (e, p) in game.scene.props.shown() {
        if inside(p.position) && !crate::destruct::adopted(e) && !p.wrecked && wanted(p.template) {
            items.push(Item {
                key: StaticKey::Entity(e),
                template: p.template,
                position: p.position,
            });
        }
    }
    // A patch at a time, nearest first.
    let patch = |p: Vec3| ((p.x / PATCH).floor() as i32, (p.z / PATCH).floor() as i32);
    items.sort_by(|a, b| {
        (patch(a.position), a.position.distance(start).to_bits())
            .cmp(&(patch(b.position), b.position.distance(start).to_bits()))
    });
    let mass = templates
        .id("M151 Softtop (VZ)")
        .and_then(|t| templates.tuning.get(&t))
        .and_then(|t| t.mass)
        .unwrap_or(1000.0);
    let rifle = &game.combat.rifle;
    let (radius, damage, least) = crate::game::blast_strength(templates);
    let blast = crate::destruct::Blast {
        at: Vec3::ZERO,
        radius,
        damage,
        least,
    };
    let hits = Hits {
        vehicle: (mass, SPEED),
        rounds: (rifle.clip.max(1), rifle.damage),
        blast: blast.at_distance(BLAST_OFF),
    };
    eprintln!(
        "props-check: {hit:?} over {} props in {area:?} (else within {CHECK_NEAR} m of {start:.0}); {hits:?}",
        items.len()
    );
    if items.is_empty() {
        return 0;
    }
    let run = |game: &mut crate::game::Game, secs: f32| {
        for _ in 0..(secs / tick).ceil() as u32 {
            game.combat.hp = hp.max(1.0e6);
            game.step(Intent::default(), tick);
        }
    };
    let mut tally: std::collections::BTreeMap<(Reaction, bool), usize> = Default::default();
    let mut unloaded = 0;
    let mut at_patch = None;
    for item in items.clone() {
        if at_patch != Some(patch(item.position)) {
            at_patch = Some(patch(item.position));
            let (px, pz) = patch(item.position);
            let middle = Vec3::new(
                (px as f32 + 0.5) * PATCH,
                item.position.y + 2.0,
                (pz as f32 + 0.5) * PATCH,
            );
            if let Some(pl) = game.scene.player.as_mut() {
                pl.drop_at(middle);
            }
            // Until this patch's models are in (or the wait is up).
            let here: Vec<StaticKey> = (items.iter())
                .filter(|i| patch(i.position) == (px, pz))
                .map(|i| i.key)
                .collect();
            let mut waited = 0.0;
            while waited < PATCH_WAIT && here.iter().any(|&k| placed_box(game, k).is_none()) {
                run(game, 0.5);
                waited += 0.5;
            }
            run(game, 0.5);
        }
        // Not in yet: her put down at it, a while longer.
        if placed_box(game, item.key).is_none() {
            if let Some(pl) = game.scene.player.as_mut() {
                pl.drop_at(item.position + Vec3::new(3.0, 2.0, 0.0));
            }
            let mut waited = 0.0;
            while waited < PATCH_WAIT && placed_box(game, item.key).is_none() {
                run(game, 0.5);
                waited += 0.5;
            }
            at_patch = None;
        }
        let Some((place, bounds)) = placed_box(game, item.key) else {
            unloaded += 1;
            eprintln!(
                "props-check: {hit:?} {:?} {:08x} at {:.0}: its model is not in",
                item.key, item.template, item.position
            );
            continue;
        };
        let before = reaction(game, item.key);
        let centre = place.transform_point3((bounds.0 + bounds.1) * 0.5);
        let half = (bounds.1 - bounds.0).length() * 0.5;
        let floor = item.position.y;
        // From her side of it.
        let forward = (centre - start).with_y(0.0).normalize_or(Vec3::Z);
        let world = game.scene.world.as_ref().expect("in play");
        let templates = &world.index.template_db;
        let makeup = Makeup::of(templates, item.template);
        let name = templates
            .names
            .get(&item.template)
            .cloned()
            .unwrap_or_else(|| format!("{:08x}", item.template));
        let row = row_name(templates, item.template);
        let mut note = String::new();
        // Already brought down (by a neighbour's hit): its reaction to
        // the same kind of hit.
        if before != Reaction::Block && before != Reaction::None {
            note = " (by a neighbour's hit)".into();
        } else {
            match hit {
                HitKind::Vehicle => {
                    let crasher = crasher_at(centre, floor, forward, mass, SPEED);
                    game.crash_with(&crasher);
                }
                HitKind::Gun => {
                    // The first way in that its rounds reach it.
                    let ways = [
                        forward,
                        -forward,
                        forward.cross(Vec3::Y),
                        Vec3::Y.cross(forward),
                    ];
                    let reached = ways.iter().find_map(|&w| {
                        let from = centre - w * SHOT_FROM;
                        let (_, _, key) =
                            world.physics.cast_ray_keyed(from, w, SHOT_FROM + half)?;
                        (key == Some(item.key)).then_some(from + w * SHOT_FROM)
                    });
                    match reached {
                        Some(at) => {
                            let strike = game.combat.rifle_strike(at, Some(item.key));
                            let n = hits.rounds.0 as usize;
                            game.hit_world_with(vec![strike; n], Vec::new());
                        }
                        None => note = " (no round reaches it)".into(),
                    }
                }
                HitKind::Blast => {
                    let at = centre - forward * (half + BLAST_OFF);
                    game.hit_world_with(Vec::new(), vec![at.with_y(floor + 0.3)]);
                }
            }
            run(game, SETTLE);
        }
        let mut got = reaction(game, item.key);
        // Standing with nothing in the way: a blast leaves it be.
        if hit == HitKind::Blast && got == Reaction::None {
            got = Reaction::Block;
        }
        let want = expected(&makeup, hit, &hits);
        let ok = want.contains(&got);
        *tally.entry((got, ok)).or_default() += 1;
        eprintln!(
            "props-check: {hit:?} {:?} {name} row {row} health {:?} physics {:?} crash {:?}: \
             {got:?}, data {want:?} -> {}{note}",
            item.key,
            makeup.health,
            makeup.physics.map(|p| (p.mass, p.heavy)),
            makeup.crash.map(|c| (c.how, c.takes)),
            if ok { "ok" } else { "MISS" }
        );
    }
    let tried: usize = tally.values().sum();
    let ok: usize = tally
        .iter()
        .filter(|((_, ok), _)| *ok)
        .map(|(_, n)| n)
        .sum();
    // Reacted at all: stood in the way, moved or broke (not gone through
    // or vanished).
    let reacted: usize = tally
        .iter()
        .filter(|((r, _), _)| matches!(r, Reaction::Block | Reaction::Move | Reaction::Break))
        .map(|(_, n)| n)
        .sum();
    eprintln!(
        "props-check summary: {hit:?}: {tried} props tried, {ok} as the data says ({:.1}%), \
         {reacted} reacted ({:.1}%), {unloaded} not loaded; {tally:?}",
        100.0 * ok as f32 / tried.max(1) as f32,
        100.0 * reacted as f32 / tried.max(1) as f32
    );
    if let Some(pl) = game.scene.player.as_mut() {
        pl.drop_at(start);
    }
    run(game, 1.0);
    game.combat.hp = hp;
    tried
}

/// Where `key` stands and its model's box, once its model has loaded.
fn placed_box(game: &crate::game::Game, key: StaticKey) -> Option<(Mat4, (Vec3, Vec3))> {
    match key {
        StaticKey::Object(i) => {
            let world = game.scene.world.as_ref()?;
            let o = world.index.objects.get(i)?;
            let m = world.object_model(i)?;
            Some((
                Mat4::from_rotation_translation(o.rotation, o.position),
                m.bounds,
            ))
        }
        StaticKey::Entity(e) => game.scene.props.placed_bounds(e),
        _ => None,
    }
}

/// How `key` stands now: one brought down is broken if its model shows a
/// wreck (or it broke into its pieces), else it vanished.
fn reaction(game: &crate::game::Game, key: StaticKey) -> Reaction {
    let Some(world) = game.scene.world.as_ref() else {
        return Reaction::None;
    };
    let props = &game.scene.props;
    let (knocked, down) = match key {
        StaticKey::Object(i) => (
            world.knocked_loose(i),
            (game.destruction.destroyed.contains(&i) || world.destroyed.contains(&i))
                .then(|| world.object_model(i).is_some_and(|m| m.has_wreck())),
        ),
        StaticKey::Entity(e) => (
            props.knocked_loose(e),
            match props.get(e) {
                None => Some(false),
                Some((p, _)) => p.wrecked.then_some(true),
            },
        ),
        _ => (None, None),
    };
    match (knocked, down) {
        (Some(true), _) | (None, Some(true)) => Reaction::Break,
        (Some(false), _) => Reaction::Move,
        (None, Some(false)) => Reaction::Vanish,
        (None, None) if world.physics.has_static(key) => Reaction::Block,
        (None, None) => Reaction::None,
    }
}
