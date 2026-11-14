//! Mines, as the game's own scripts run them
//! ([`m2_game::mission::Mission::placed_mines`] has the scripts' side).
//!
//! - A placed mine (`Mine (Human)`, `(Vehicle)`, `(IED)`, `(Water)
//!   (Light)`) has `Health` 1 and `PhysicsPropertyCrashable` (from
//!   samples): a vehicle running into it or a blast reaching it kills it.
//!   Its script hears of the death (`mine.OnDeath`) and, a moment later,
//!   spawns its explosion. A person stepping on a human mine is the
//!   script's own (it kills itself).
//! - Her proximity mine, once armed, becomes `Proximity Mine (Planted)`
//!   (the thrown mine's `TimerResponse`, 6 s): an object of the scripts'
//!   world whose script pops it up as a bounding grenade when someone
//!   comes within 6 m, and removes it.
//! - What the scripts spawn that is an explosion (`Explosion (AT Mine)`)
//!   goes off where it is spawned.
//!
//! Invented: how near a vehicle must come to a placed mine to run into it
//! ([`CRASH_REACH`]); a blast's reach over a mine is the props' (as for
//! the contracts' targets, [`crate::mission`]'s `BLAST_REACH`); that an
//! explosion template spawned by a script goes off at once (it has an
//! `Explosive` and nothing else to set it off; not traced in the 360
//! code).

use m2_game::mission::Mission;
use m2_math::glam::Vec3;

use m2_engine::weapons::Blast;

use crate::combat::Combat;
use crate::game::Game;

/// How near a vehicle's middle comes to a placed mine, flat, for it to
/// run into it, metres. Invented: about half a car's length.
pub const CRASH_REACH: f32 = 2.5;

/// The blast of explosion template `what` (`"Explosion (AT Mine)"`), if
/// it is one.
pub fn explosion_blast(db: &m2_engine::Templates, what: &str) -> Option<Blast> {
    let id = db.id(what)?;
    let e = db.explosions.get(&id)?;
    Some(Blast {
        template: id,
        radius: e.radius,
        damage: e.damage,
        min_damage: e.least,
        max_force: e.force,
        key: (e.kind != 0).then_some(e.kind),
    })
}

/// Whether a placed mine at `mine` is run into by a vehicle at one of
/// `vehicles` or reached by a blast at one of `blasts` (within
/// `blast_reach`).
pub fn set_off(mine: Vec3, vehicles: &[Vec3], blasts: &[Vec3], blast_reach: f32) -> bool {
    vehicles.iter().any(|v| {
        v.with_y(0.0).distance(mine.with_y(0.0)) <= CRASH_REACH && (v.y - mine.y).abs() < 3.0
    }) || blasts.iter().any(|b| b.distance(mine) <= blast_reach)
}

/// The placed mines run into or blasted since the last look die (their
/// script then sets them off); her armed mines become the scripts'.
pub fn report(game: &mut Game, mission: &mut Mission, blasts: &[Vec3], blast_reach: f32) {
    let vehicles: Vec<Vec3> = game
        .scene
        .world
        .as_ref()
        .map(|w| {
            game.scene
                .vehicles
                .iter()
                .filter_map(|v| w.physics.vehicle_transform(v.id?))
                .map(|m| m.w_axis.truncate())
                .collect()
        })
        .unwrap_or_default();
    for (g, at) in mission.placed_mines() {
        if set_off(Vec3::from(at), &vehicles, blasts, blast_reach) {
            eprintln!("mission: mine {g:#x} set off");
            mission.kill(g);
        }
    }
    planted(&mut game.combat, game.scene.world.as_ref(), mission);
}

/// Her armed mines as objects of the scripts' world; one its script took
/// away (it popped up) goes from her charges.
fn planted(combat: &mut Combat, world: Option<&m2_engine::World>, mission: &mut Mission) {
    combat.scripted_mines = Mission::runs_mine_scripts();
    if !combat.scripted_mines {
        return;
    }
    let lying: Vec<(u32, u32, Vec3)> = combat.mines_planted().collect();
    for &(id, template, at) in &lying {
        if combat.mine_guids.contains_key(&id) {
            continue;
        }
        let Some(name) = world.and_then(|w| w.index.template_db.names.get(&template).cloned())
        else {
            continue;
        };
        let g = mission.ammo_spawned(&name, at.to_array());
        eprintln!("mission: her mine is planted, {name} ({g:#x})");
        combat.mine_guids.insert(id, g);
    }
    let mut popped = Vec::new();
    combat.mine_guids.retain(|id, g| {
        let alive = mission.world().borrow().objects.contains_key(g);
        let lies = lying.iter().any(|l| l.0 == *id);
        if !alive {
            eprintln!("mission: her mine {g:#x} pops up");
            popped.push(*id);
        } else if !lies {
            mission.ammo_gone(*g);
        }
        alive && lies
    });
    for id in popped {
        combat.remove_charge(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vehicle_on_it_or_a_blast_near_it_sets_a_mine_off() {
        let mine = Vec3::new(10.0, 0.0, 10.0);
        assert!(set_off(mine, &[Vec3::new(11.5, 1.0, 11.0)], &[], 10.0));
        assert!(!set_off(mine, &[Vec3::new(14.0, 1.0, 10.0)], &[], 10.0));
        // A vehicle passing over it high up (a helicopter) does not.
        assert!(!set_off(mine, &[Vec3::new(10.0, 20.0, 10.0)], &[], 10.0));
        assert!(set_off(mine, &[], &[Vec3::new(18.0, 0.0, 10.0)], 10.0));
        assert!(!set_off(mine, &[], &[Vec3::new(30.0, 0.0, 10.0)], 10.0));
    }
}
