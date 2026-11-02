//! A faction's pursuit brings reinforcements: while the engine's pursuit
//! runs ([`m2_game::api::PursuitClock`]), the pursuing army sends its
//! units up to the count its pursuit table gives for her state and the
//! pursuit's level; its pursuit helicopter flies in and sets down its
//! soldiers near her, who fight her.
//!
//! From the 360 code: the pursuit manager's tick (0x822371F8) tops each
//! kind of unit up to `table[faction][her state][kind][level]` less those
//! of that kind it already has out, while a pursuit is active and the kind
//! isn't held back; the counts come from the table `e480240b` (columns
//! `Faction`, `Player Is`, `Spawn Type`, `Threat1..3`, loaded at
//! 0x82237790); her state is her vehicle's kind (a helicopter, a tank, a
//! boat), else how she stands to the roads (0x822644E0): on or within
//! [`NEAR_ROAD`] of one `Stopped` (`Driving` in a vehicle going fast
//! enough), off them `OffRoad`. From the scripts: the faction manager
//! starts a pursuit only when a faction at its lowest standing has a
//! witness report her (`FinishedReporting` -> `IncrementPursuit`), for 5 s
//! at the first level (`SetPursuitSeconds(faction, 5, true)`), and lets
//! the next report come 30 s on at the soonest. So the Venezuelans send a
//! helicopter only to a report while she is off the roads (one at
//! `Threat1`), in a helicopter, a tank or a boat, never on a timer, and
//! none while one of theirs is still out.
//!
//! From the data: each army has one pursuit helicopter, its template
//! named `(Pursuit)`, crewed by its own (`Rider` records: the Allies'
//! `MH53J (Pursuit)` carries four `Allied Airborne`, the Chinese
//! `Ka29b (pursuit)` three soldiers and a machine gunner, the UP's
//! `Coanda Transport (Pursuit)`, the Guerrillas' `UH1 Transport
//! (Pursuit)`, the Pirates' and the Venezuelans' `Alouette3 Transport`);
//! the Venezuelans' table `4d5bf92a` picks `Alouette3 Transport (VZ)
//! (Pursuit)` at `Threat1`.
//!
//! Not yet sent: the tables' cars, tanks, boats and men, and their
//! helicopters other than the transport (gunships at higher levels).
//! Inferred: that a helicopter is "out" until its soldiers are all down
//! (the 360 counts its units alive). Invented: from how far it comes
//! ([`FROM`], [`CRUISE_ABOVE`], [`SET_DOWN`]), and that its soldiers are
//! simply put down on the ground under it as it hovers (no rope or
//! landing).

use std::collections::HashMap;

use m2_assets::animation_table::AnimationTable;
use m2_assets::name_hash;
use m2_math::glam::Vec3;

use crate::flights::{Copter, Flights, Release};

/// The pursuit unit counts' table (`Faction`, `Player Is`, `Spawn Type`,
/// `Threat1..3`), in the resident block.
pub const COUNTS_TABLE: u32 = 0xe480_240b;

/// Metres from a road within which she counts as on it (from the 360
/// code: the constant 20.0 at 0x82B7C710 that 0x822644E0 compares her
/// distance to).
pub const NEAR_ROAD: f32 = 20.0;

/// Where she is, as the pursuit table's `Player Is` reads it (the 360's
/// names, at 0x82B7C698).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Heli,
    Tank,
    Boat,
    OffRoad,
    Stopped,
    Driving,
}

impl Standing {
    fn name(self) -> &'static str {
        match self {
            Self::Heli => "Heli",
            Self::Tank => "Tank",
            Self::Boat => "Boat",
            Self::OffRoad => "OffRoad",
            Self::Stopped => "Stopped",
            Self::Driving => "Driving",
        }
    }
}

/// The table's name for a faction (by its abbreviation,
/// [`crate::npc::faction_of`]). The Allies' row (`8baef8f3`) is known by
/// its hash alone.
fn faction_key(faction: &str) -> Option<u32> {
    Some(match faction {
        "Vza" => name_hash("VZ"),
        "Chi" => name_hash("Chi"),
        "Gur" => name_hash("Gur"),
        "Oil" => name_hash("OC"),
        "Pir" => name_hash("Pir"),
        "All" => 0x8bae_f8f3,
        _ => return None,
    })
}

/// How many of each kind of unit a pursuit sends ([`COUNTS_TABLE`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts(HashMap<(u32, u32, u32), [i32; 3]>);

impl Counts {
    /// The table's rows; empty if it hasn't the columns.
    pub fn of(t: &AnimationTable) -> Self {
        let cols = [
            "Faction",
            "Player Is",
            "Spawn Type",
            "Threat1",
            "Threat2",
            "Threat3",
        ]
        .map(|c| t.column(c));
        let [Some(f), Some(p), Some(s), Some(a), Some(b), Some(c)] = cols else {
            return Self::default();
        };
        let mut out = HashMap::new();
        for r in 0..t.rows() {
            let cell = |c| t.cell(r, c).unwrap_or(0);
            // Clamped as 0x82237790 does.
            let n = |c| (cell(c) as i32).clamp(0, 64);
            out.insert((cell(f), cell(p), cell(s)), [n(a), n(b), n(c)]);
        }
        Self(out)
    }

    /// Rows read.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether none were read.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Helicopters `faction` sends at `level` (0 to 2) while she is
    /// `standing` so.
    pub fn helicopters(&self, faction: &str, standing: Standing, level: usize) -> usize {
        let Some(f) = faction_key(faction) else {
            return 0;
        };
        self.0
            .get(&(f, name_hash(standing.name()), name_hash("Heli")))
            .and_then(|n| n.get(level))
            .map_or(0, |&n| n.max(0) as usize)
    }
}

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
    sent: Vec<Sent>,
    next: u32,
    /// Helicopters sent and soldiers brought, for the log's tally.
    pub flown: usize,
    pub brought: usize,
}

impl Reinforcements {
    /// The helicopters `faction` has out (on their way, or their soldiers
    /// still coming into play).
    pub fn out(&self, faction: &str) -> usize {
        self.sent
            .iter()
            .filter(|s| s.faction == faction && !s.called_off)
            .count()
    }

    /// Runs on by `dt`: `wanted`, the pursuing faction and how many
    /// helicopters its pursuit wants out now (none without a pursuit),
    /// `her` where she is. Starts the helicopters due in `flights` (those
    /// wanted beyond those out) and returns the soldiers due in play
    /// (guid, template, where).
    pub fn step(
        &mut self,
        wanted: Option<(&'static str, usize)>,
        squads: &HashMap<&'static str, Squad>,
        flights: &mut Flights,
        her: Vec3,
        dt: f32,
    ) -> Vec<(u32, String, Vec3)> {
        let due = wanted
            .filter(|(f, _)| squads.contains_key(f))
            .map_or(0, |(f, n)| n.saturating_sub(self.out(f)));
        for _ in 0..due {
            let Some((f, _)) = wanted else { break };
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

    /// The contract is over: the helicopters still flying set nobody
    /// down. Invented (ours landed soldiers in the villa she had just
    /// taken).
    pub fn call_off(&mut self) {
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
    fn a_pursuit_sends_the_helicopters_it_wants_out_and_no_more() {
        let squads = squads_of_two();
        let mut r = Reinforcements::default();
        let mut flights = Flights::default();
        let her = Vec3::new(100.0, 5.0, 100.0);
        // None without a pursuit, nor for a faction without a helicopter.
        assert!(r.step(None, &squads, &mut flights, her, 1.0).is_empty());
        r.step(Some(("Oil", 1)), &squads, &mut flights, her, 1.0);
        assert_eq!(r.flown, 0);
        // One wanted: one sent, however long the pursuit wants it.
        r.step(Some(("Vza", 1)), &squads, &mut flights, her, 0.1);
        assert_eq!(r.flown, 1);
        for _ in 0..100 {
            r.step(Some(("Vza", 1)), &squads, &mut flights, her, 1.0);
        }
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
        let due = r.step(None, &squads, &mut flights, her, 0.1);
        assert_eq!(due.len(), 2);
        assert!(
            due.iter()
                .all(|(_, t, p)| t == "VZ Soldier" && p.distance(at) < 3.0)
        );
        for (g, _, _) in &due {
            r.brought(*g);
        }
        assert_eq!(r.brought, 2);
        assert!(r.step(None, &squads, &mut flights, her, 0.1).is_empty());
        assert_eq!(r.out("Vza"), 0);
        // Its soldiers down, a pursuit still wanting one sends another.
        r.step(Some(("Vza", 1)), &squads, &mut flights, her, 0.1);
        assert_eq!(r.flown, 2);
    }

    #[test]
    fn one_called_off_in_flight_sets_nobody_down_and_keeps_its_release() {
        let squads = squads_of_two();
        let mut r = Reinforcements::default();
        let mut flights = Flights::default();
        let her = Vec3::new(100.0, 5.0, 100.0);
        r.step(Some(("Vza", 1)), &squads, &mut flights, her, 0.5);
        assert_eq!(r.flown, 1);
        let pilot = r.sent[0].pilot;
        r.call_off();
        // Still about till it hovers, so its release is not the scripts'.
        assert!(r.step(None, &squads, &mut flights, her, 0.1).is_empty());
        let left = r.take_releases(vec![Release {
            pilot,
            to: her,
            dropped: Vec::new(),
        }]);
        assert!(left.is_empty());
        assert!(r.step(None, &squads, &mut flights, her, 0.1).is_empty());
        assert!(r.sent.is_empty());
        assert_eq!(r.brought, 0);
    }

    // The table's counts: the Venezuelans send one helicopter at the
    // first level off the roads and none by them on foot.
    #[test]
    fn the_counts_table_gives_each_army_its_helicopters_by_her_standing() {
        let mut c = Counts::default();
        let key = |f: &str, p: &str| (name_hash(f), name_hash(p), name_hash("Heli"));
        c.0.insert(key("VZ", "OffRoad"), [1, 2, 3]);
        c.0.insert(key("VZ", "Stopped"), [0, 1, 1]);
        assert_eq!(c.helicopters("Vza", Standing::OffRoad, 0), 1);
        assert_eq!(c.helicopters("Vza", Standing::OffRoad, 2), 3);
        assert_eq!(c.helicopters("Vza", Standing::Stopped, 0), 0);
        assert_eq!(c.helicopters("Chi", Standing::OffRoad, 0), 0);
        assert_eq!(c.helicopters("Civ", Standing::OffRoad, 0), 0);
    }
}
