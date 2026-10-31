//! What the state layers place for her to take, break open or man: the
//! health packs, ammunition boxes and weapons lying about a contract's
//! ground (`vz_state_vza_con001_post`'s 5 `Health Pickup`s and 7 `Ammo
//! Pickup (Bullet)`s, `vz_state_pmccon001`'s 9 `Shotgun`s on the villa's
//! tables), its supply crates (`Supply Drop (VZ)`, `Supply Drop
//! (Health)`) and its emplaced guns (`Emplaced MG (VZ)`).
//!
//! Each stands as a prop ([`Props`]) while its layer is in. A pickup also
//! lies in [`Flights`] to be taken as a crate's leavings are (drawn by its
//! prop, which is taken down when she takes it); a crate breaks open as a
//! delivered one does, to blows, rounds and blasts, and leaves its
//! template's `SpawnOnDeath` pickups; an emplaced gun is manned with the
//! use key ([`crate::combat::Manned`]). Which template is which comes from
//! the template database: a `Pickup` record, a `SpawnOnDeath` naming a
//! pickup, the `Emplacedweapon` label with a `PhysicalLink`ed weapon (PC
//! data, 2026-10-10). Money (cash, fuel) is a pickup too, walked over, and
//! a crate of it (`Cash (Large)`) breaks open to leave it (PC data,
//! 2026-10-14).

use std::collections::HashMap;

use m2_engine::{Physics, StaticKey, Templates};
use m2_math::glam::{Quat, Vec3};

use crate::combat::{Manned, Strike};
use crate::flights::{Flights, Leaves, Pickup};
use crate::props::Props;

/// A placed crate's health when its template gives none (every `Supply
/// Drop` gives 100).
const CRATE_HP: f32 = 100.0;
/// A blast this near breaks a crate open, metres (as a delivered one).
const CRATE_BLAST: f32 = 6.0;
/// What a crate leaves lies this far apart across it, metres (as a
/// delivered one's: its template gives no places).
const LEFT_APART: f32 = 0.7;
/// How near she must stand to an emplaced gun to man it, metres
/// (invented: its `Vehicle Entrance (_Default)` gives a hardpoint, no
/// reach).
pub const MAN_REACH: f32 = 2.0;
/// Manning one, she stands this far behind its pivot, metres (invented:
/// its entrance hardpoint isn't read yet; playtest 4 had her fire it from
/// wherever she stood, beside it).
pub const MAN_BEHIND: f32 = 0.8;

/// Where she stands to man a gun turning about `pivot`, facing `facing`
/// (radians, 0 along +Z), her feet at height `feet_y`: behind it.
pub fn stand_behind(pivot: Vec3, facing: f32, feet_y: f32) -> Vec3 {
    let ahead = Vec3::new(facing.sin(), 0.0, facing.cos());
    (pivot - ahead * MAN_BEHIND).with_y(feet_y)
}
/// The keys of what a placed crate leaves (its entity in the low bits,
/// which stay below `0x2000_0000`; the leaving's place in bits 29-30):
/// apart from crates' guids and soldiers' drops (`0x7000_0000 + n`).
const LEFT: u32 = 0x8000_0000;
/// `--placed-check` tries what stands within this of her, metres (the
/// first contract's ground is about 1,300 m across, from its boat start).
const CHECK_NEAR: f32 = 1500.0;

/// What a placed template is to her.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Taken: health, ammunition, a weapon.
    Pickup,
    /// Broken open for what it leaves.
    Crate,
    /// Manned.
    Emplaced,
}

/// What template `t` is to her, if anything.
pub fn kind(templates: &Templates, t: u32) -> Option<Kind> {
    if templates.leavings.pickups.contains_key(&t) {
        return Some(Kind::Pickup);
    }
    let emplaced = templates
        .leavings
        .labels
        .get(&t)
        .is_some_and(|l| l.contains(&m2_assets::name_hash("Emplacedweapon")));
    if emplaced && templates.arms.armaments.contains_key(&t) {
        return Some(Kind::Emplaced);
    }
    templates
        .leavings
        .spawns
        .get(&t)?
        .iter()
        .any(|s| templates.leavings.pickups.contains_key(s))
        .then_some(Kind::Crate)
}

/// The way a placed rotation faces across the ground (its +z), radians.
fn yaw_of(rotation: Quat) -> f32 {
    let f = rotation * Vec3::Z;
    f.x.atan2(f.z)
}

/// Where emplaced gun `m`'s rounds start ([`Manned::barrel`]): its
/// `PhysicalLink`'s hardpoint on its own model, placed as its prop stands
/// (world), and its weapon model's `Muzzle` node from that model's origin.
/// From samples: both nodes (PmcCon031's launchers' hardpoint stands over
/// their sandbags, the muzzle out along the weapon's +z).
pub fn barrel<R: std::io::Read + std::io::Seek>(
    source: Option<&mut m2_engine::AssetSource<R>>,
    props: &Props,
    m: &Manned,
) -> Option<(Vec3, Vec3)> {
    let source = source?;
    let (p, _) = props.get(m.entity)?;
    let mut node = |model: u32, name: u32| {
        let data = source.model(model).ok()?;
        let parsed = data.parse().ok()?;
        m2_assets::vehicle::marker(&parsed, name).map(Vec3::from)
    };
    let mount = node(m.model, m.hardpoint);
    let muzzle = m
        .weapon
        .model
        .and_then(|w| node(w, m2_engine::vehicle::GUN_MUZZLE));
    eprintln!(
        "placed: emplaced gun {:#x} model {:08x}: hardpoint {:08x} at {mount:?}, weapon {:?} \
         muzzle {muzzle:?}",
        m.entity, m.model, m.hardpoint, m.weapon.model
    );
    Some((p.position + p.rotation * mount?, muzzle?))
}

/// How many of each are in play, for the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub pickups: usize,
    pub crates: usize,
    pub emplaced: usize,
}

/// The placed ones in play.
#[derive(Debug, Default)]
pub struct Placed {
    /// Each one standing: its kind and template.
    known: HashMap<u32, (Kind, u32)>,
    /// Crates' health left.
    hp: HashMap<u32, f32>,
    /// Seconds to the next look over the props ([`SYNC_EVERY`]).
    wait: f32,
}

/// How often what stands is looked over, seconds: there are some 10,000
/// props, too many to go through every tick.
const SYNC_EVERY: f32 = 0.5;

impl Placed {
    /// Takes in what now stands and lets go of what no longer does (its
    /// layer out, taken, broken): a pickup lies to be taken where it
    /// stands, drawn by its prop. Returns what was taken in.
    pub fn sync(
        &mut self,
        props: &Props,
        flights: &mut Flights,
        templates: &Templates,
        dt: f32,
    ) -> Counts {
        self.wait -= dt;
        if self.wait > 0.0 {
            return Counts::default();
        }
        self.wait = SYNC_EVERY;
        let shown: HashMap<u32, (u32, Vec3, Quat)> = props
            .shown()
            .filter(|(_, p)| !p.wrecked && p.template != 0)
            .map(|(e, p)| (e, (p.template, p.position, p.rotation)))
            .collect();
        self.known.retain(|e, (kind, t)| {
            let stands = shown.contains_key(e);
            if !stands && *kind == Kind::Pickup {
                flights.forget((*e, *t));
            }
            stands
        });
        self.hp.retain(|e, _| shown.contains_key(e));
        let mut taken = Counts::default();
        for (&e, &(t, at, rotation)) in &shown {
            if self.known.contains_key(&e) {
                continue;
            }
            let Some(kind) = kind(templates, t) else {
                continue;
            };
            self.known.insert(e, (kind, t));
            match kind {
                Kind::Pickup => {
                    let Some(what) = crate::mission::pickup(templates, t) else {
                        continue;
                    };
                    taken.pickups += 1;
                    flights.leave(Pickup {
                        from: e,
                        // Its prop draws it.
                        what: Leaves {
                            model: None,
                            ..what
                        },
                        at,
                        yaw: yaw_of(rotation),
                    });
                }
                Kind::Crate => {
                    taken.crates += 1;
                    let hp = templates.durability.get(&t).map_or(CRATE_HP, |d| d.health);
                    self.hp.insert(e, hp);
                }
                Kind::Emplaced => taken.emplaced += 1,
            }
        }
        taken
    }

    /// Whether `entity` is a placed pickup (taken: its prop comes down).
    pub fn is_pickup(&self, entity: u32) -> bool {
        self.known.get(&entity).is_some_and(|k| k.0 == Kind::Pickup)
    }

    /// She took placed pickup `entity`: its prop comes down.
    pub fn taken(&mut self, entity: u32, props: &mut Props, physics: &mut Physics) {
        if self.known.remove(&entity).is_some() {
            props.hide(physics, [entity]);
        }
    }

    /// Blows and rounds (`struck`) and blasts reach the placed crates:
    /// one taken to no health or caught in a blast breaks open (drawn as
    /// its wreck, or gone) and leaves its `SpawnOnDeath` pickups side by
    /// side across where it stood. Returns those broken (entity, where).
    pub fn strike(
        &mut self,
        props: &mut Props,
        physics: &mut Physics,
        flights: &mut Flights,
        templates: &Templates,
        struck: &[Strike],
        blasts: &[Vec3],
    ) -> Vec<(u32, Vec3)> {
        let mut broken = Vec::new();
        for (&e, hp) in &mut self.hp {
            let Some(at) = props.centre(e) else {
                continue;
            };
            *hp -= struck
                .iter()
                .filter(|s| s.key == Some(StaticKey::Entity(e)))
                .map(|s| s.damage)
                .sum::<f32>();
            if blasts.iter().any(|b| b.distance(at) < CRATE_BLAST) {
                *hp = 0.0;
            }
            if *hp <= 0.0 {
                broken.push(e);
            }
        }
        let mut opened = Vec::new();
        for e in broken {
            self.hp.remove(&e);
            let (Some((_, t)), Some((p, _))) = (self.known.remove(&e), props.get(e)) else {
                continue;
            };
            let (at, yaw) = (p.position, yaw_of(p.rotation));
            props.wreck(physics, e, None);
            let left = leavings(templates, t);
            eprintln!(
                "placed: crate {} ({e:#x}) at {at:.1} broken open: leaves [{}]",
                templates.names.get(&t).map_or("?", String::as_str),
                left.iter()
                    .map(|l| templates.names.get(&l.template).map_or("?", String::as_str))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let across = Quat::from_rotation_y(yaw) * Vec3::X;
            let middle = (left.len() as f32 - 1.0) * 0.5;
            for (i, what) in left.into_iter().enumerate() {
                flights.leave(Pickup {
                    from: LEFT | ((i as u32 & 3) << 29) | e,
                    what,
                    at: at + across * ((i as f32 - middle) * LEFT_APART),
                    yaw,
                });
            }
            opened.push((e, at));
        }
        opened
    }

    /// Where the emplaced gun stands that is nearest `to` of those within
    /// `seek` metres of `from` and `reach` metres of `to` (flat
    /// distances), of those standing where `keep` holds: one to man to fire
    /// on `to`.
    pub fn emplacement_for(
        &self,
        props: &Props,
        from: Vec3,
        to: Vec3,
        seek: f32,
        reach: f32,
        keep: impl Fn(Vec3) -> bool,
    ) -> Option<Vec3> {
        let flat = |a: Vec3, b: Vec3| (a - b).with_y(0.0).length();
        self.known
            .iter()
            .filter(|(_, k)| k.0 == Kind::Emplaced)
            .filter_map(|(&e, _)| Some(props.get(e)?.0.position))
            .filter(|&p| flat(p, from) <= seek && flat(p, to) <= reach && keep(p))
            .min_by(|a, b| flat(*a, to).total_cmp(&flat(*b, to)))
    }

    /// How many emplaced guns are known, and the nearest to `at` that
    /// stands as a prop (entity and flat distance), for the log.
    pub fn emplaced_nearest(&self, props: &Props, at: Vec3) -> (usize, Option<(u32, f32)>) {
        let known: Vec<u32> = self
            .known
            .iter()
            .filter(|(_, k)| k.0 == Kind::Emplaced)
            .map(|(&e, _)| e)
            .collect();
        let nearest = known
            .iter()
            .filter_map(|&e| Some((e, (props.get(e)?.0.position - at).with_y(0.0).length())))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        (known.len(), nearest)
    }

    /// The emplaced gun nearest `at` within [`MAN_REACH`], as she would
    /// man it: its weapon (the template's first `PhysicalLink`ed one the
    /// arsenal knows), its turn and tilt (its `Turret`), all its rounds.
    pub fn emplacement_near(
        &self,
        props: &Props,
        templates: &Templates,
        at: Vec3,
    ) -> Option<Manned> {
        let (e, t, p) = self
            .known
            .iter()
            .filter(|(_, k)| k.0 == Kind::Emplaced)
            .filter_map(|(&e, &(_, t))| Some((e, t, props.get(e)?.0)))
            .map(|(e, t, p)| (e, t, p, (p.position - at).with_y(0.0).length()))
            .filter(|&(.., d)| d <= MAN_REACH)
            .min_by(|a, b| a.3.total_cmp(&b.3))
            .map(|(e, t, p, _)| (e, t, *p))?;
        let (weapon, hardpoint) = templates
            .arms
            .armaments
            .get(&t)?
            .mounts
            .iter()
            .find_map(|m| Some((templates.arms.weapons.get(&m.weapon)?, m.hardpoint)))?;
        let weapon = weapon.clone();
        let turret = templates.tuning.get(&t).and_then(|v| v.turret);
        let (lo, hi) = (-90f32.to_radians(), 90f32.to_radians());
        Some(Manned {
            entity: e,
            template: t,
            pivot: p.position + Vec3::Y * crate::combat::MANNED_PIVOT,
            facing: yaw_of(p.rotation),
            yaw: turret.and_then(|t| t.yaw).unwrap_or((lo, hi)),
            pitch: turret.map_or((-0.35, 0.5), |t| t.pitch),
            rounds: weapon.clip.max(1) * (1 + weapon.reserve_clips),
            weapon,
            model: p.model,
            hardpoint,
            barrel: None,
        })
    }

    /// How many of each are in play.
    pub fn counts(&self) -> Counts {
        let n = |k: Kind| self.known.values().filter(|v| v.0 == k).count();
        Counts {
            pickups: n(Kind::Pickup),
            crates: n(Kind::Crate),
            emplaced: n(Kind::Emplaced),
        }
    }
}

/// With `--placed-check` (testing): tries each placed pickup, crate and
/// emplaced gun in play not tried before, as she would come to it, and
/// logs what happened (`placed-check:` lines): a pickup with her emptied
/// out (health 1, no spare rounds, no grenades) so that it gives
/// something, stood on (the use key pressed where it is worded); a crate
/// struck hard, then what it left taken likewise; a gun manned, fired for
/// two seconds and left. She goes back where she stood, as she was.
pub fn check(game: &mut crate::game::Game, tried: &mut std::collections::HashSet<u32>, tick: f32) {
    use crate::mission::Offer;
    use crate::player::Intent;
    let Some(start) = game.scene.player.as_ref().map(|p| p.character.position) else {
        return;
    };
    let (hp, rounds, launcher, grenades) = (
        game.combat.hp,
        game.combat.rifle_rounds,
        game.combat.launcher_rounds,
        game.combat.grenades,
    );
    // Those near her (the contract's ground), not the whole island's.
    let props = &game.scene.props;
    let mut todo: Vec<(u32, Kind, u32)> = game
        .placed
        .known
        .iter()
        .filter(|(e, _)| !tried.contains(e))
        .filter(|(e, _)| {
            props
                .get(**e)
                .is_some_and(|(p, _)| p.position.distance(start) < CHECK_NEAR)
        })
        .map(|(&e, &(k, t))| (e, k, t))
        .collect();
    todo.sort_unstable_by_key(|t| t.0);
    let name = |game: &crate::game::Game, t: u32| {
        game.scene
            .world
            .as_ref()
            .and_then(|w| w.index.template_db.names.get(&t).cloned())
            .unwrap_or_else(|| format!("{t:#x}"))
    };
    // Put down at `at`, she waits for the ground there to stream in and
    // to land on it (at most 20 s).
    let go = |game: &mut crate::game::Game, at: Vec3| {
        if let Some(pl) = game.scene.player.as_mut() {
            pl.drop_at(at);
        }
        for k in 0..(20.0 / tick) as u32 {
            if k > 10 && game.scene.player.as_ref().is_some_and(|p| p.landed) {
                break;
            }
            game.step(Intent::default(), tick);
        }
    };
    let run = |game: &mut crate::game::Game, intent: Intent, ticks: u32| {
        for k in 0..ticks {
            game.step(
                if k == 0 {
                    intent
                } else {
                    Intent {
                        use_pressed: false,
                        ..intent
                    }
                },
                tick,
            );
        }
    };
    // The use key's words where she stands, once shown (within 2 s).
    let worded = |game: &mut crate::game::Game| {
        for _ in 0..(2.0 / tick) as u32 {
            if let Some(p) = game.hud_info(None).prompt {
                return p;
            }
            game.step(Intent::default(), tick);
        }
        String::new()
    };
    // Takes what lies under `key` as she would: on it, the use key if it
    // is worded. Returns the prompt and whether it went.
    let take = |game: &mut crate::game::Game, key: (u32, u32)| -> Option<(Offer, String, bool)> {
        let p = *game
            .scene
            .flights
            .pickups()
            .iter()
            .find(|p| p.key() == key)?;
        game.combat.hp = 1.0;
        game.combat.rifle_rounds.stored = 0;
        if let Some(r) = &mut game.combat.launcher_rounds {
            r.stored = 0;
        }
        game.combat.grenades = 0;
        let world = game.scene.world.as_ref()?;
        let offer = crate::mission::offer(&world.index.template_db, &game.combat, &p.what);
        go(game, p.at);
        let prompt = worded(game);
        run(
            game,
            Intent {
                use_pressed: offer != Offer::Walk,
                ..Intent::default()
            },
            10,
        );
        let gone = !game.scene.flights.pickups().iter().any(|q| q.key() == key);
        Some((offer, prompt, gone))
    };
    for (e, kind, t) in todo {
        tried.insert(e);
        match kind {
            Kind::Pickup => match take(game, (e, t)) {
                Some((offer, prompt, gone)) => {
                    let down = game.scene.props.get(e).is_none();
                    eprintln!(
                        "placed-check: pickup {} ({e:#x}) {offer:?} prompt {prompt:?}: taken {gone}, prop down {down}",
                        name(game, t)
                    );
                }
                None => eprintln!("placed-check: pickup {} ({e:#x}): not lying", name(game, t)),
            },
            Kind::Crate => {
                let Some(at) = game.scene.props.centre(e) else {
                    continue;
                };
                let blow = [Strike {
                    at,
                    damage: 1.0e4,
                    key: Some(StaticKey::Entity(e)),
                    round: 0.0,
                    kind: 0,
                }];
                let opened = match (&mut game.scene.world, &mut game.placed) {
                    (Some(world), placed) => placed.strike(
                        &mut game.scene.props,
                        &mut world.physics,
                        &mut game.scene.flights,
                        &world.index.template_db,
                        &blow,
                        &[],
                    ),
                    _ => Vec::new(),
                };
                let left: Vec<(u32, u32)> = game
                    .scene
                    .flights
                    .pickups()
                    .iter()
                    .filter(|p| p.from & !(LEFT | 3 << 29) == e && p.from & LEFT == LEFT)
                    .map(Pickup::key)
                    .collect();
                let mut taken = Vec::new();
                for key in &left {
                    if let Some((offer, prompt, gone)) = take(game, *key) {
                        taken.push(format!(
                            "{} {offer:?} {prompt:?} taken {gone}",
                            name(game, key.1)
                        ));
                    }
                }
                eprintln!(
                    "placed-check: crate {} ({e:#x}) broken {}: {} left: [{}]",
                    name(game, t),
                    !opened.is_empty(),
                    left.len(),
                    taken.join("; ")
                );
            }
            Kind::Emplaced => {
                let Some((p, _)) = game.scene.props.get(e) else {
                    continue;
                };
                let (at, facing) = (p.position, yaw_of(p.rotation));
                go(game, at - Vec3::new(facing.sin(), 0.0, facing.cos()));
                let prompt = worded(game);
                let off = game.scene.player.as_ref().map_or(f32::NAN, |p| {
                    (p.character.position - at).with_y(0.0).length()
                });
                run(
                    game,
                    Intent {
                        use_pressed: true,
                        ..Intent::default()
                    },
                    2,
                );
                let before = game.combat.manned.as_ref().map(|m| m.rounds);
                let hits = game.combat.hits;
                let mut fire = Intent::default();
                fire.trigger.fire = true;
                run(game, fire, (2.0 / tick) as u32);
                let after = game.combat.manned.as_ref().map(|m| m.rounds);
                eprintln!(
                    "placed-check: emplaced {} ({e:#x}) prompt {prompt:?} {off:.1} m off: manned {}, rounds {before:?} -> {after:?}, hits {}",
                    name(game, t),
                    before.is_some(),
                    game.combat.hits - hits
                );
                game.combat.manned = None;
            }
        }
    }
    go(game, start);
    game.combat.hp = hp;
    game.combat.rifle_rounds = rounds;
    game.combat.launcher_rounds = launcher;
    game.combat.grenades = grenades;
}

/// What a crate of template `t` leaves: its `SpawnOnDeath` pickups.
pub fn leavings(templates: &Templates, t: u32) -> Vec<Leaves> {
    templates
        .leavings
        .spawns
        .get(&t)
        .into_iter()
        .flatten()
        .filter_map(|&s| crate::mission::pickup(templates, s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_engine::pickup::{CASH, EQUIPMENT, HEALTH, Leavings, PickupInfo};

    #[test]
    fn she_mans_a_gun_from_behind_it() {
        // A gun facing +Z at the origin: she stands on -Z, on her feet.
        let at = stand_behind(Vec3::new(0.0, 1.1, 0.0), 0.0, -0.2);
        assert!((at - Vec3::new(0.0, -0.2, -MAN_BEHIND)).length() < 1e-5);
        // Facing +X: she stands on -X.
        let at = stand_behind(Vec3::ZERO, std::f32::consts::FRAC_PI_2, 0.0);
        assert!((at - Vec3::new(-MAN_BEHIND, 0.0, 0.0)).length() < 1e-5);
    }

    fn info(kind: u32, heals: u32, gives: Vec<(u32, u32)>) -> PickupInfo {
        PickupInfo {
            kind,
            radius: 1.0,
            ammo: gives.first().map(|g| g.0),
            rounds: gives.first().map_or(0, |g| g.1),
            walked: gives.clone(),
            gives,
            heals,
            icon: None,
            effect: None,
        }
    }

    /// The PC data's templates (2026-10-10): `Health Pickup` 80006ea9,
    /// `Shotgun` 800056a1 (its id here invented), `Supply Drop (Health)`
    /// 8000855d leaving two health packs, `Cash (Large)` leaving cash,
    /// `Emplaced MG (VZ)` 8000855a.
    fn db() -> Templates {
        let mut db = Templates::default();
        for (id, name) in [
            (0x8000_6ea9, "Health Pickup"),
            (0x8000_56a1, "Shotgun"),
            (0x8000_855d, "Supply Drop (Health)"),
            (0x8000_a277, "Pickup (Cash)"),
            (0x8000_a278, "Cash (Large)"),
            (0x8000_855a, "Emplaced MG (VZ)"),
        ] {
            db.names.insert(id, name.to_owned());
        }
        db.leavings = Leavings {
            pickups: HashMap::from([
                (0x8000_6ea9, info(HEALTH, 50, Vec::new())),
                (0x8000_56a1, info(EQUIPMENT, 0, vec![(0x8000_56a0, 8)])),
                // `Cash (Medium)`'s record: type 5, 5000 in its rounds.
                (
                    0x8000_a277,
                    PickupInfo {
                        rounds: 5000,
                        ..info(CASH, 0, Vec::new())
                    },
                ),
            ]),
            spawns: HashMap::from([
                (0x8000_855d, vec![0x8000_6ea9, 0x8000_6ea9]),
                (0x8000_a278, vec![0x8000_a277]),
            ]),
            labels: HashMap::from([(0x8000_855a, vec![m2_assets::name_hash("Emplacedweapon")])]),
            ..Leavings::default()
        };
        db.arms
            .armaments
            .insert(0x8000_855a, m2_engine::weapons::Armament::default());
        db
    }

    #[test]
    fn the_template_database_says_what_each_placed_one_is() {
        let db = db();
        assert_eq!(kind(&db, 0x8000_6ea9), Some(Kind::Pickup));
        assert_eq!(kind(&db, 0x8000_56a1), Some(Kind::Pickup));
        assert_eq!(kind(&db, 0x8000_855d), Some(Kind::Crate));
        assert_eq!(kind(&db, 0x8000_855a), Some(Kind::Emplaced));
        // Money is a pickup, and a crate of it a crate.
        assert_eq!(kind(&db, 0x8000_a277), Some(Kind::Pickup));
        assert_eq!(kind(&db, 0x8000_a278), Some(Kind::Crate));
        assert_eq!(kind(&db, 0x1234), None);
    }

    #[test]
    fn a_health_crate_leaves_both_its_packs() {
        let db = db();
        let left = leavings(&db, 0x8000_855d);
        assert_eq!(left.len(), 2);
        assert!(
            left.iter()
                .all(|l| l.template == 0x8000_6ea9 && l.health == 50.0)
        );
    }

    #[test]
    fn a_crate_of_cash_leaves_cash_walked_over() {
        let db = db();
        let left = leavings(&db, 0x8000_a278);
        assert_eq!(left.len(), 1);
        assert!(left[0].is_money());
        assert_eq!(
            (left[0].cash, left[0].fuel, left[0].weapon),
            (5000, 0, None)
        );
        let combat = crate::combat::Combat::default();
        assert_eq!(
            crate::mission::offer(&db, &combat, &left[0]),
            crate::mission::Offer::Walk
        );
    }

    #[test]
    fn a_placed_rotation_faces_its_plus_z() {
        assert!(yaw_of(Quat::IDENTITY).abs() < 1e-6);
        let quarter = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!((yaw_of(quarter) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }
}
