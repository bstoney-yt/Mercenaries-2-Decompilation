//! A faction's pursuit brings reinforcements: as an army takes up the
//! chase (she hurt one of theirs, or their witness reported her), its
//! pursuit helicopter flies in and sets down its soldiers near her, who
//! fight her.
//!
//! From the data: each army has one pursuit helicopter, its template
//! named `(Pursuit)`, crewed by its own (`Rider` records: the Allies'
//! `MH53J (Pursuit)` carries four `Allied Airborne`, the Chinese
//! `Ka29b (pursuit)` three soldiers and a machine gunner, the UP's
//! `Coanda Transport (Pursuit)`, the Guerrillas' `UH1 Transport
//! (Pursuit)`, the Pirates' and the Venezuelans' `Alouette3 Transport`).
//! The game's text says reinforcements come when she is reported
//! (`Fiona.Cam.32`: "The VZ reported you, get ready for reinforcements!").
//!
//! Invented: when and from how far it comes ([`DELAY`], [`FROM`],
//! [`CRUISE_ABOVE`], [`SET_DOWN`]), at most one a pursuit and one a
//! faction per [`COOLDOWN`], and that its soldiers are simply put down on
//! the ground under it as it hovers (no rope or landing).

use std::collections::HashMap;

use m2_math::glam::Vec3;

use crate::flights::{Copter, Flights, Release};

/// Seconds from a pursuit starting to the helicopter setting out.
pub const DELAY: f32 = 15.0;
/// Seconds before that faction sends another.
pub const COOLDOWN: f32 = 120.0;
/// How far off it sets out, how high over her it flies, and how far from
/// her it sets its soldiers down, metres.
pub const FROM: f32 = 600.0;
pub const CRUISE_ABOVE: f32 = 70.0;
pub const SET_DOWN: f32 = 35.0;
/// Seconds its soldiers wait to come into play (their models loading)
/// before they are given up.
const BRING_FOR: f32 = 30.0;

/// The pilots' and soldiers' guids, out of the scripts' way.
const FIRST_GUID: u32 = 0xfe00_0000;

/// One army's pursuit helicopter, as the data gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Squad {
    pub template: String,
    pub model: Option<u32>,
    pub bank: Option<u32>,
    /// The soldiers it brings (its riders but its pilots), by template.
    pub soldiers: Vec<String>,
}

/// The pursuit helicopters of the armies (by the faction's abbreviation,
/// [`crate::npc::faction_of`]) among `passengers` (template name, lower
/// case, to the templates of its other riders) and `drivers`: every
/// `(pursuit)` template whose riders are one army's.
pub fn squads(
    passengers: &HashMap<String, Vec<String>>,
    drivers: &HashMap<String, String>,
    templates: &m2_engine::world::Templates,
) -> HashMap<&'static str, Squad> {
    let mut out = HashMap::new();
    let mut names: Vec<&String> = passengers
        .keys()
        .filter(|n| n.contains("(pursuit)"))
        .collect();
    names.sort();
    for name in names {
        let mut soldiers: Vec<String> = passengers[name]
            .iter()
            .filter(|t| !t.to_ascii_lowercase().contains("pilot"))
            .cloned()
            .collect();
        // The driver rides among them too where he is a soldier.
        if let Some(d) = drivers.get(name)
            && let Some(i) = soldiers.iter().position(|s| s == d)
        {
            soldiers.remove(i);
        }
        let Some(faction) = soldiers.first().and_then(|s| crate::npc::faction_of(s)) else {
            continue;
        };
        let t = templates.get(name);
        out.entry(faction).or_insert(Squad {
            template: name.clone(),
            model: t.and_then(|t| t.model),
            bank: t.and_then(|t| t.engine),
            soldiers,
        });
    }
    out
}

/// One on its way, or whose soldiers are coming into play.
#[derive(Clone, Debug, PartialEq)]
struct Sent {
    pilot: u32,
    faction: &'static str,
    soldiers: Vec<(u32, String)>,
    /// Where they are set down, once it hovers there, and seconds since.
    down: Option<(Vec3, f32)>,
    /// Called off in flight ([`Reinforcements::call_off`]): it sets
    /// nobody down.
    called_off: bool,
}

/// The reinforcements under way.
#[derive(Debug, Default)]
pub struct Reinforcements {
    /// Seconds each faction's pursuit has run, till it sends.
    waiting: HashMap<&'static str, f32>,
    /// Seconds before each may send again.
    resting: HashMap<&'static str, f32>,
    sent: Vec<Sent>,
    next: u32,
    /// Helicopters sent and soldiers brought, for the log's tally.
    pub flown: usize,
    pub brought: usize,
}

impl Reinforcements {
    /// Runs on by `dt`: `pursuing` the factions chasing her, `her` where
    /// she is. Starts the helicopters due in `flights` and returns the
    /// soldiers due in play (guid, template, model wanted, where).
    pub fn step(
        &mut self,
        pursuing: &[&'static str],
        squads: &HashMap<&'static str, Squad>,
        flights: &mut Flights,
        her: Vec3,
        dt: f32,
    ) -> Vec<(u32, String, Vec3)> {
        for t in self.resting.values_mut() {
            *t -= dt;
        }
        self.resting.retain(|_, t| *t > 0.0);
        self.waiting.retain(|f, _| pursuing.contains(f));
        for &f in pursuing {
            if self.resting.contains_key(f) || !squads.contains_key(f) {
                continue;
            }
            let t = self.waiting.entry(f).or_insert(0.0);
            *t += dt;
            if *t < DELAY {
                continue;
            }
            self.waiting.remove(f);
            self.resting.insert(f, COOLDOWN);
            let squad = &squads[f];
            let pilot = FIRST_GUID + self.next;
            self.next += 1;
            let soldiers = squad
                .soldiers
                .iter()
                .map(|s| {
                    self.next += 1;
                    (FIRST_GUID + self.next, s.clone())
                })
                .collect();
            // From a way that turns with each one sent.
            let a = self.next as f32 * 2.4;
            let way = Vec3::new(a.sin(), 0.0, a.cos());
            let from = her + way * FROM + Vec3::Y * CRUISE_ABOVE;
            let to = her + way * SET_DOWN;
            flights.start(
                Copter {
                    guid: pilot,
                    pilot,
                    model: squad.model,
                    bank: squad.bank,
                },
                Vec::new(),
                from,
                to,
            );
            eprintln!(
                "reinforcements: {f} send their {} from {from:.0} with {:?}",
                squad.template, squad.soldiers
            );
            self.flown += 1;
            self.sent.push(Sent {
                pilot,
                faction: f,
                soldiers,
                down: None,
                called_off: false,
            });
        }
        let mut out = Vec::new();
        for s in &mut self.sent {
            let Some((at, since)) = &mut s.down else {
                continue;
            };
            *since += dt;
            for (k, (g, t)) in s.soldiers.iter().enumerate() {
                let a = k as f32 * 1.9;
                out.push((*g, t.clone(), *at + Vec3::new(a.sin(), 0.0, a.cos()) * 2.5));
            }
        }
        // One called off stays till its release is taken (it is not the
        // scripts' to hear of).
        self.sent.retain(|s| {
            if s.called_off {
                s.down.is_none()
            } else {
                s.down.is_none_or(|(_, t)| t < BRING_FOR) && !s.soldiers.is_empty()
            }
        });
        out
    }

    /// The contract is over: nothing more is sent, and the helicopters
    /// still flying set nobody down. Invented: the game's own pursuit
    /// helicopters are not read; ours landed soldiers in the villa she had
    /// just taken.
    pub fn call_off(&mut self) {
        self.waiting.clear();
        for s in self.sent.iter_mut().filter(|s| s.down.is_none()) {
            s.called_off = true;
            s.soldiers.clear();
        }
    }

    /// Takes the releases that are its helicopters' (their soldiers are
    /// set down there), leaving the rest for the scripts.
    pub fn take_releases(&mut self, releases: Vec<Release>) -> Vec<Release> {
        releases
            .into_iter()
            .filter(|r| {
                let Some(s) = self.sent.iter_mut().find(|s| s.pilot == r.pilot) else {
                    return true;
                };
                if s.called_off {
                    eprintln!(
                        "reinforcements: the {} helicopter was called off",
                        s.faction
                    );
                    s.down = Some((r.to, BRING_FOR));
                    return false;
                }
                eprintln!(
                    "reinforcements: the {} helicopter sets its soldiers down at {:.1}",
                    s.faction, r.to
                );
                s.down = Some((r.to, 0.0));
                false
            })
            .collect()
    }

    /// Soldier `guid` came into play.
    pub fn brought(&mut self, guid: u32) {
        for s in &mut self.sent {
            let before = s.soldiers.len();
            s.soldiers.retain(|(g, _)| *g != guid);
            self.brought += before - s.soldiers.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn squads_of_two() -> HashMap<&'static str, Squad> {
        let squad = |t: &str, s: &[&str]| Squad {
            template: t.into(),
            model: None,
            bank: None,
            soldiers: s.iter().map(|s| (*s).to_owned()).collect(),
        };
        HashMap::from([
            (
                "Vza",
                squad(
                    "alouette3 transport (vz) (pursuit)",
                    &["VZ Soldier", "VZ Soldier"],
                ),
            ),
            ("Chi", squad("ka29b (pursuit)", &["Chinese Soldier"])),
        ])
    }

    #[test]
    fn the_data_gives_each_army_its_pursuit_helicopter_and_its_soldiers() {
        let passengers = HashMap::from([
            (
                "mh53j (pursuit)".to_owned(),
                vec![
                    "Allied Pilot".into(),
                    "Allied Pilot".into(),
                    "Allied Airborne".into(),
                ],
            ),
            (
                "uh1 transport (pursuit)".to_owned(),
                vec!["Guerilla Soldier".into(), "Guerilla Soldier".into()],
            ),
            ("uh1 transport".to_owned(), vec!["Guerilla Soldier".into()]),
        ]);
        let drivers = HashMap::from([(
            "uh1 transport (pursuit)".to_owned(),
            "Guerilla Tank Commander".to_owned(),
        )]);
        let s = squads(
            &passengers,
            &drivers,
            &m2_engine::world::Templates::default(),
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s["All"].soldiers, ["Allied Airborne"]);
        assert_eq!(s["Gur"].soldiers.len(), 2);
        assert_eq!(s["Gur"].template, "uh1 transport (pursuit)");
    }

    #[test]
    fn a_pursuit_sends_one_helicopter_whose_soldiers_come_down_by_her() {
        let squads = squads_of_two();
        let mut r = Reinforcements::default();
        let mut flights = Flights::default();
        let her = Vec3::new(100.0, 5.0, 100.0);
        // Not before the delay, nor for a faction without a helicopter.
        assert!(
            r.step(&["Vza", "Oil"], &squads, &mut flights, her, DELAY - 1.0)
                .is_empty()
        );
        assert_eq!(r.flown, 0);
        r.step(&["Vza", "Oil"], &squads, &mut flights, her, 1.5);
        assert_eq!(r.flown, 1);
        // Once a cooldown, however long the pursuit.
        r.step(&["Vza"], &squads, &mut flights, her, DELAY + 1.0);
        assert_eq!(r.flown, 1);
        // Its release is its own; another's goes to the scripts.
        let pilot = r.sent[0].pilot;
        let at = her + Vec3::X * SET_DOWN;
        let mine = Release {
            pilot,
            to: at,
            dropped: Vec::new(),
        };
        let theirs = Release {
            pilot: 7,
            to: at,
            dropped: Vec::new(),
        };
        let left = r.take_releases(vec![mine, theirs]);
        assert_eq!(left.iter().map(|r| r.pilot).collect::<Vec<_>>(), [7]);
        let due = r.step(&["Vza"], &squads, &mut flights, her, 0.1);
        assert_eq!(due.len(), 2);
        assert!(
            due.iter()
                .all(|(_, t, p)| t == "VZ Soldier" && p.distance(at) < 3.0)
        );
        for (g, _, _) in &due {
            r.brought(*g);
        }
        assert_eq!(r.brought, 2);
        assert!(r.step(&["Vza"], &squads, &mut flights, her, 0.1).is_empty());
    }

    #[test]
    fn one_called_off_in_flight_sets_nobody_down_and_keeps_its_release() {
        let squads = squads_of_two();
        let mut r = Reinforcements::default();
        let mut flights = Flights::default();
        let her = Vec3::new(100.0, 5.0, 100.0);
        r.step(&["Vza"], &squads, &mut flights, her, DELAY + 0.5);
        assert_eq!(r.flown, 1);
        let pilot = r.sent[0].pilot;
        r.call_off();
        // Still about till it hovers, so its release is not the scripts'.
        assert!(r.step(&[], &squads, &mut flights, her, 0.1).is_empty());
        let left = r.take_releases(vec![Release {
            pilot,
            to: her,
            dropped: Vec::new(),
        }]);
        assert!(left.is_empty());
        assert!(r.step(&[], &squads, &mut flights, her, 0.1).is_empty());
        assert!(r.sent.is_empty());
        assert_eq!(r.brought, 0);
    }
}
