//! Mines, as the game's own scripts run them
//! ([`m2_game::mission::Mission::placed_mines`] has the scripts' side).
//!
//! - A placed mine (`Mine (Human)`, `(Vehicle)`, `(IED)`, `(Water)
//!   (Light)`) has `Health` 1 and `PhysicsPropertyCrashable` (from
//!   samples): a vehicle running into it or a blast reaching it kills it.
//!   Its script hears of the death (`mine.OnDeath`) and, a moment later,
//!   spawns its explosion. A person stepping on a human mine is the
//!   script's own (it kills itself).
//! - It dies as any state layer prop does, by the port's crash and wear
//!   rules ([`crate::destruct::Destruction`]), not by a reach of its own:
//!   its `PhysicsPropertyCrashable` is `Flattened` (type 2) taking 0 kg,
//!   so any vehicle whose body or box meets it brings it down (the 360's
//!   vehicle contact with a crashable body, posted as 0x2003 on
//!   0x8322DC38 by 0x821858C0); a blast wears its 1 health by the
//!   blast's damage at its distance times its `ModifierKey` row's cell
//!   (the 360's health change 0x822889F8). From samples, the land mines'
//!   row (1abb43fd) takes nothing of a frag grenade's or an AT mine's
//!   blast (no chain of mines going off), all of a bunker buster's stage
//!   and 0.04 of a 25 mm shell's; the water mines' row (d9bc5bda) all of
//!   an AT mine's.
//! - Her proximity mine, once armed, becomes `Proximity Mine (Planted)`
//!   (the thrown mine's `TimerResponse`, 6 s): an object of the scripts'
//!   world whose script pops it up as a bounding grenade when someone
//!   comes within 6 m, and removes it.
//! - What the scripts spawn that is an explosion (`Explosion (AT Mine)`)
//!   goes off where it is spawned.
//!
//! Invented: that an explosion template spawned by a script goes off at once (it has an
//! `Explosive` and nothing else to set it off; not traced in the 360
//! code).

use m2_game::mission::Mission;
use m2_math::glam::Vec3;

use m2_engine::weapons::Blast;

use crate::combat::Combat;
use crate::game::Game;

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

/// Whether a placed mine's prop (`shown`: wrecked, knocked loose; `None`
/// while it is not shown) was brought down: run into or worn through.
pub fn set_off(shown: Option<(bool, bool)>) -> bool {
    shown.is_some_and(|(wrecked, knocked)| wrecked || knocked)
}

/// The placed mines run into or blasted since the last look die (their
/// script then sets them off); her armed mines become the scripts'.
pub fn report(game: &mut Game, mission: &mut Mission) {
    let props = &game.scene.props;
    for (g, _) in mission.placed_mines() {
        let shown = props
            .get(g)
            .map(|(p, _)| (p.wrecked, props.knocked_loose(g).is_some()));
        if set_off(shown) {
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

/// What `blasts` do, all told, to a prop of template `template` whose
/// box (`centre`, half its diagonal `half`) stands where it does, as
/// [`crate::destruct::Destruction::blast`] wears it: each at its distance
/// from the box, times the template's `ModifierKey` cell for its kind.
pub fn blast_wear(
    db: &m2_engine::Templates,
    template: u32,
    (centre, half): (Vec3, f32),
    blasts: &[crate::destruct::Blast],
) -> f32 {
    let at: Vec<(f32, u32)> = blasts
        .iter()
        .map(|b| {
            let d = (centre.distance(b.at) - half).max(0.0);
            (b.at_distance(d), b.kind)
        })
        .collect();
    crate::breakables::Makeup::blasted(db, template, &at)
}

/// `--mine-check`: once the scripts run and a placed mine is in the
/// layers, she is put [`CHECK_OFF`] m from the nearest one; once its prop
/// is shown, an RPG goes off [`CHECK_BLAST`] m from its box (logged with
/// whether its data says that wears its health through: the land mines'
/// row takes nothing of an RPG's blast, the water mines' a fifth), and,
/// should it stand, a jeep at 10 m/s runs into it
/// ([`crate::breakables::crasher_at`]). `mine-check: PASS` once the mine
/// went as its data says and its object is dead in the scripts' world
/// ([`CHECK_WAIT`] s on), else `FAIL`.
#[derive(Debug, Default)]
pub struct MineCheck {
    t: f32,
    mine: Option<u32>,
    shown_at: Option<f32>,
    blasted: Option<(bool, bool)>,
    crashed_at: Option<f32>,
    done: bool,
}

/// How far from the mine she is put, metres.
pub const CHECK_OFF: f32 = 12.0;
/// How far from the mine's box the RPG goes off, metres.
pub const CHECK_BLAST: f32 = 1.0;
/// Seconds the mine's death is waited for after each hit (its script's
/// 0.25 s, and more), and for its prop to be shown.
pub const CHECK_WAIT: f32 = 3.0;
/// Seconds the check waits for the scripts' placed mines, and for her
/// mine's prop to stream in.
pub const CHECK_GIVE_UP: f32 = 120.0;

impl MineCheck {
    fn end(&mut self, pass: bool, what: String) {
        self.done = true;
        let verdict = if pass { "PASS" } else { "FAIL" };
        eprintln!("mine-check: {verdict} {what}");
    }

    pub fn tick(&mut self, game: &mut Game, mission: &mut Mission, dt: f32) {
        if self.done {
            return;
        }
        self.t += dt;
        let alive = |m: &Mission, g: u32| m.placed_mines().iter().any(|p| p.0 == g);
        let Some(g) = self.mine else {
            let here = game.focus();
            let nearest = mission.placed_mines().into_iter().min_by(|a, b| {
                let d = |p: [f32; 3]| Vec3::from(p).distance(here);
                d(a.1).total_cmp(&d(b.1))
            });
            match nearest {
                Some((g, at)) => {
                    let at = Vec3::from(at);
                    eprintln!("mine-check: mine {g:#x} at {at:.1}, she is put {CHECK_OFF} m off");
                    if let Some(p) = game.scene.player.as_mut() {
                        p.driving = None;
                        p.drop_at(at + Vec3::new(CHECK_OFF, 2.0, 0.0));
                    }
                    game.on_foot = true;
                    self.mine = Some(g);
                    self.t = 0.0;
                }
                None if self.t > CHECK_GIVE_UP => {
                    self.end(false, format!("no placed mine in {CHECK_GIVE_UP} s"));
                }
                None => {}
            }
            return;
        };
        let props = &game.scene.props;
        let shown = props
            .placed_bounds(g)
            .zip(props.get(g).map(|(p, _)| p.template));
        let Some(world) = game.scene.world.as_ref() else {
            return;
        };
        let db = &world.index.template_db;
        match (self.blasted, shown) {
            (None, Some(((at, (lo, hi)), template))) => {
                let centre = at.transform_point3((lo + hi) * 0.5);
                let half = (hi - lo).length() * 0.5;
                let from = centre + Vec3::X * (half + CHECK_BLAST);
                let blasts = crate::game::rocket_blasts(db, from);
                let wear = blast_wear(db, template, (centre, half), &blasts);
                let durability = db.durability.get(&template).map_or(0.0, |d| d.health);
                let goes = wear >= durability;
                eprintln!(
                    "mine-check: an RPG {CHECK_BLAST} m off: {wear:.1} of its {durability}                      health; it should {}",
                    if goes { "go off" } else { "stand" }
                );
                game.hit_world_with(Vec::new(), vec![from]);
                self.blasted = Some((goes, false));
                self.shown_at = Some(self.t);
            }
            (None, None) if self.t > CHECK_GIVE_UP => {
                self.end(
                    false,
                    format!("mine {g:#x}'s prop not shown in {CHECK_GIVE_UP} s"),
                );
            }
            (None, None) => {}
            (Some((goes, _)), _) if self.crashed_at.is_none() => {
                if self.t - self.shown_at.unwrap_or(0.0) < CHECK_WAIT {
                    return;
                }
                let gone = !alive(mission, g);
                if goes != gone {
                    let did = if gone { "went off" } else { "stood" };
                    self.end(false, format!("the RPG's blast: it {did}"));
                } else if gone {
                    self.end(true, "the RPG's blast set it off, as its data says".into());
                } else if let Some(((at, (lo, hi)), _)) = shown {
                    let centre = at.transform_point3((lo + hi) * 0.5);
                    let floor = at.w_axis.y;
                    let jeep = crate::breakables::crasher_at(
                        centre,
                        floor,
                        Vec3::Z,
                        (1000.0, 10.0),
                        crate::breakables::JEEP_BOX,
                    );
                    eprintln!(
                        "mine-check: it stood the blast, as its data says; a jeep runs into it"
                    );
                    game.crash_with(&jeep);
                    self.blasted = Some((goes, true));
                    self.crashed_at = Some(self.t);
                } else {
                    self.end(false, "its prop went, but not its object".into());
                }
            }
            (Some(_), _) => {
                if self.t - self.crashed_at.unwrap_or(0.0) < CHECK_WAIT {
                    return;
                }
                if alive(mission, g) {
                    self.end(false, "the jeep ran into it and it stands".into());
                } else {
                    self.end(
                        true,
                        "it stood the blast as its data says; the jeep set it off".into(),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mine_goes_off_once_its_prop_is_brought_down() {
        assert!(!set_off(None), "not shown: nothing has met it");
        assert!(!set_off(Some((false, false))));
        assert!(set_off(Some((true, false))), "wrecked");
        assert!(set_off(Some((false, true))), "knocked loose");
    }

    #[test]
    fn a_blast_wears_a_mine_by_its_distance_from_the_box_and_its_cell() {
        use crate::destruct::Blast;
        use m2_engine::durability::BlastPush;
        let db = m2_engine::Templates::default();
        let blast = |at: Vec3, kind: u32| Blast {
            at,
            radius: 5.0,
            damage: 200.0,
            least: 0.0,
            kind,
            push: BlastPush::none(at),
        };
        // A box 0.5 m across its diagonal at the origin; the blast 1.25 m
        // from its middle is 1 m from it: 200 x (1 - 1/5).
        let mine = (Vec3::ZERO, 0.25);
        let at = Vec3::X * 1.25;
        assert!((blast_wear(&db, 7, mine, &[blast(at, 0)]) - 160.0).abs() < 1e-3);
        // A kind its template has no row for: none of it.
        assert_eq!(blast_wear(&db, 7, mine, &[blast(at, 0x1693_0afe)]), 0.0);
        // Past its reach: nothing.
        assert_eq!(blast_wear(&db, 7, mine, &[blast(Vec3::X * 6.0, 0)]), 0.0);
    }
}
