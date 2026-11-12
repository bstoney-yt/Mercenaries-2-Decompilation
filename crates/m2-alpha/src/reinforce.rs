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
//! From the 360 code: it comes in as every air unit of a pursuit does
//! (0x82239DA0 -> 0x8223ADD0, [`crate::gunships::coming_in`]): 350 m off
//! about the way she faces, 70 m over the ground there, facing her.
//!
//! From the scripts: once in, the helicopter is its own script's
//! (`pursuitcopter`, the `ObjectScript` of every `(Pursuit)` template):
//! as it wakes it asks for a drop zone by her (`Ai.TestDropZone`), lands
//! there (`Ai.Goal` of `HeliLand`), has its passengers get out
//! (`Ai.Deploy`) and goes home (`MrxSupport.GoHome`); with no drop zone
//! it comes 50 m over her and asks again 3 s on, three times, then goes
//! home. So this module only decides when one is due and spawns it into
//! the scripts' world ([`m2_game::mission::Mission::spawn`]); the game
//! flies its landing as it does a soldier delivery's.
//!
//! Not yet sent: the tables' men, and their helicopters other than the
//! transport and the gunships.
//! Inferred: that a helicopter is "out" while it is in the scripts' world
//! (the 360 counts its units alive). Invented: that a contract's end
//! takes away the helicopters whose passengers are still aboard
//! ([`Reinforcements::call_off`]).

use std::collections::HashMap;

use m2_assets::animation_table::AnimationTable;
use m2_assets::name_hash;

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
    /// Its name in the factions' pursuit tables' `When` column (a
    /// helicopter `Helicopter` there).
    pub fn table_name(self) -> &'static str {
        match self {
            Self::Heli => "Helicopter",
            other => other.name(),
        }
    }

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
        self.units(faction, standing, "Heli", level)
    }

    /// Units of `kind` (the table's `Spawn Type`: `Car`, `Tank`, `Boat`,
    /// `Heli`, `human`) `faction` sends at `level` while she is `standing`
    /// so.
    pub fn units(&self, faction: &str, standing: Standing, kind: &str, level: usize) -> usize {
        let Some(f) = faction_key(faction) else {
            return 0;
        };
        self.0
            .get(&(f, name_hash(standing.name()), name_hash(kind)))
            .and_then(|n| n.get(level))
            .map_or(0, |&n| n.max(0) as usize)
    }
}

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

/// One in the scripts' world.
#[derive(Clone, Debug, PartialEq)]
struct Sent {
    copter: u32,
    faction: &'static str,
    /// Called off ([`Reinforcements::call_off`]): no longer counted, and
    /// to be taken away if its passengers are still aboard.
    called_off: bool,
}

/// The reinforcements under way.
#[derive(Debug, Default)]
pub struct Reinforcements {
    sent: Vec<Sent>,
    /// Called off and not yet looked at ([`Reinforcements::take_off`]).
    off: Vec<u32>,
    /// Helicopters sent, for the log's tally.
    pub flown: usize,
}

impl Reinforcements {
    /// The helicopters `faction` has out.
    pub fn out(&self, faction: &str) -> usize {
        self.sent
            .iter()
            .filter(|s| s.faction == faction && !s.called_off)
            .count()
    }

    /// The helicopters due now: for `wanted`, the pursuing faction and
    /// how many helicopters its pursuit wants out (none without a
    /// pursuit), its pursuit helicopter's template once for each wanted
    /// beyond those out.
    pub fn due(
        &self,
        wanted: Option<(&'static str, usize)>,
        squads: &HashMap<&'static str, Squad>,
    ) -> Vec<(&'static str, String)> {
        let Some((f, n)) = wanted else {
            return Vec::new();
        };
        let Some(squad) = squads.get(f) else {
            return Vec::new();
        };
        (self.out(f)..n)
            .map(|_| (f, squad.template.clone()))
            .collect()
    }

    /// `faction`'s helicopter was spawned into the scripts' world as
    /// `copter`.
    pub fn sent(&mut self, faction: &'static str, copter: u32) {
        self.flown += 1;
        self.sent.push(Sent {
            copter,
            faction,
            called_off: false,
        });
    }

    /// Forgets the helicopters no longer in the scripts' world (`there`:
    /// whether a guid still is): gone home, or taken away.
    pub fn keep(&mut self, there: impl Fn(u32) -> bool) {
        self.sent.retain(|s| there(s.copter));
    }

    /// The contract is over: its helicopters count no more, and are to be
    /// taken away if their passengers are still aboard
    /// ([`Reinforcements::take_off`]). Invented (ours landed soldiers in
    /// the villa she had just taken).
    pub fn call_off(&mut self) {
        for s in self.sent.iter_mut().filter(|s| !s.called_off) {
            s.called_off = true;
            self.off.push(s.copter);
        }
    }

    /// The helicopters called off since the last look.
    pub fn take_off(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.off)
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
        // None without a pursuit, nor for a faction without a helicopter.
        assert!(r.due(None, &squads).is_empty());
        assert!(r.due(Some(("Oil", 1)), &squads).is_empty());
        // One wanted: its army's own helicopter, once.
        let due = r.due(Some(("Vza", 1)), &squads);
        assert_eq!(
            due,
            [("Vza", "alouette3 transport (vz) (pursuit)".to_owned())]
        );
        r.sent("Vza", 0x4000_0001);
        assert_eq!(r.flown, 1);
        assert_eq!(r.out("Vza"), 1);
        // However long the pursuit wants it, no second while it is out.
        assert!(r.due(Some(("Vza", 1)), &squads).is_empty());
        // Two wanted: one more.
        assert_eq!(r.due(Some(("Vza", 2)), &squads).len(), 1);
        // Gone from the scripts' world (home), a pursuit still wanting one
        // sends another.
        r.keep(|g| g != 0x4000_0001);
        assert_eq!(r.out("Vza"), 0);
        assert_eq!(r.due(Some(("Vza", 1)), &squads).len(), 1);
    }

    #[test]
    fn one_called_off_counts_no_more_and_is_named_once() {
        let squads = squads_of_two();
        let mut r = Reinforcements::default();
        r.sent("Vza", 7);
        r.call_off();
        assert_eq!(r.out("Vza"), 0);
        // Named once, and forgotten when it is gone.
        r.call_off();
        assert_eq!(r.take_off(), [7]);
        assert!(r.take_off().is_empty());
        assert_eq!(r.due(Some(("Vza", 1)), &squads).len(), 1);
        r.keep(|_| false);
        assert!(r.sent.is_empty());
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
