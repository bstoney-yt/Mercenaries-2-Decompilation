//! The Achievements page: the game's achievement list, read at run time
//! from the scripts in the player's install, and for each one whether a
//! save shows it earned, where the scripts' save data can show that.
//!
//! # Provenance
//!
//! **From the 360 scripts** (bytecode in the install's `vz.wad`; nothing
//! of it is committed):
//!
//! - The list is `MrxAchievements.tAchievementsList` (script
//!   [`SCRIPT`]): one `{sName, nRequiredCount}` per achievement, in the
//!   order the page keeps. [`read_list`] reads it from the script's
//!   bytecode as the launcher starts; the ids are the scripts' own.
//! - Multiplayer, left out: `ACHIEVEMENT_PLAY_COOP`, and
//!   `ACHIEVEMENT_DAMAGE_INC`, which `MrxAchievements.ProcessSupportEvent`
//!   counts only while `Net.IsActive()` (co-op), in `tSupportUsedInCoop`.
//! - Earned in a save ([`RULES`]), each where its grant is:
//!   - The five faction ones: `MrxFactionManager.SetRelation(faction,
//!     "Pmc", n)` (script `b2cec5f5`) grants the faction's
//!     `sMaxRelationAchievement` once its relation to the PMC is at
//!     `_knRelationMax` (100): Allied Nations `STAND_UP_AND_SHOUT`, China
//!     `LONGING_FOR_FIRE`, UPLA `FOREVER_FREE`, Universal Petroleum
//!     `DIRTY_DEEDS`, Pirates `ISLAND_DOMINATION`. A save whose
//!     `tFactionData.tRelations[faction].Pmc` is 100 shows it.
//!   - `SCHOOLS_OUT`: granted in `VzaCon001`'s `PostGate` (script
//!     `650b5d57`), on the way to its first checkpoint; a save with that
//!     contract finished shows it.
//!   - `GONE_SHOOTIN`: granted just before `MrxTaskContract.Complete` in
//!     each of `PmcCon018`, `PmcCon031` to `PmcCon034` (scripts
//!     `2a2114bc`, `3330ba97`, `49331bd0`, `333537c5`, `2923dee6`); a save
//!     with any of them finished shows it.
//!   - `RAGE`: granted with `EnableResourceCounters(true)` (script
//!     `ce4166ed`), the flag the save keeps as `bResourceCountersEnabled`
//!     (`WifMissionFlow.AreResourceCountersEnabled`). That this is the
//!     only call turning it on is **from samples** (the other call found
//!     turns it off).
//!   - A contract is finished when `tFlowData.tMyFlowData` has its id
//!     ([`crate::save_stats`]).
//!
//! What is **not tracked**: the 360 keeps earned achievements in the
//! player's profile, not in a save, so a save can only show those whose
//! condition it keeps. The counted ones (`Pg.AchievementAddCount`) keep
//! their counts in the profile (+0x348C, past the record's end); the
//! others are granted on a condition inside a mission or by the engine,
//! which a save does not record. A save that does not show one says
//! "not yet in this save", never "locked": it may have been earned in
//! another save.
//!
//! **Invented**: the titles (the ids' words, capitalised) and each
//! line's wording of the condition.

use m2_assets::lua::{Chunk, Constant, Function, Instruction};

use crate::save_stats::Value;

/// `MrxAchievements`' script, by its name hash.
pub const SCRIPT: u32 = 0x0fcc_e0ec;

/// The ids' prefix.
const PREFIX: &str = "ACHIEVEMENT_";

/// The multiplayer ones, left off the page.
pub const MULTIPLAYER: [&str; 2] = ["ACHIEVEMENT_PLAY_COOP", "ACHIEVEMENT_DAMAGE_INC"];

/// One achievement of the scripts' list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    /// `nRequiredCount`: how many times its event must happen.
    pub required: u32,
}

impl Entry {
    /// The id's words, capitalised: `ACHIEVEMENT_WILD_ONE` is "Wild One".
    pub fn title(&self) -> String {
        title(&self.id)
    }
}

pub fn title(id: &str) -> String {
    id.strip_prefix(PREFIX)
        .unwrap_or(id)
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| {
                f.to_uppercase()
                    .chain(c.flat_map(char::to_lowercase))
                    .collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const OP_SETTABLE: u8 = 9;
/// Lua 5.1's RK: a register, or a constant from 256.
const RK_CONSTANT: u32 = 256;

fn rk(f: &Function, x: u32) -> Option<&Constant> {
    x.checked_sub(RK_CONSTANT)
        .and_then(|k| f.constants.get(k as usize))
}

/// `tAchievementsList` from `MrxAchievements`' main function: each
/// `SETTABLE t "sName" "<id>"` starts an entry, the next `SETTABLE t
/// "nRequiredCount" <n>` gives its count. The multiplayer ones are kept
/// here; [`single_player`] leaves them out.
pub fn read_list(chunk: &Chunk) -> Vec<Entry> {
    let f = &chunk.main;
    let mut out: Vec<Entry> = Vec::new();
    for &word in &f.code {
        let i = Instruction::decode51(word);
        if i.op != OP_SETTABLE {
            continue;
        }
        match (rk(f, i.b), rk(f, i.c)) {
            (Some(Constant::String(k)), Some(Constant::String(v)))
                if k == "sName" && v.starts_with(PREFIX) =>
            {
                out.push(Entry {
                    id: v.clone(),
                    required: 1,
                });
            }
            (Some(Constant::String(k)), Some(Constant::Number(n))) if k == "nRequiredCount" => {
                if let Some(e) = out.last_mut() {
                    e.required = n.max(1.0) as u32;
                }
            }
            _ => {}
        }
    }
    out
}

/// The list without the multiplayer ones.
pub fn single_player(list: Vec<Entry>) -> Vec<Entry> {
    list.into_iter()
        .filter(|e| !MULTIPLAYER.contains(&e.id.as_str()))
        .collect()
}

/// What shows an achievement earned in a save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Any of these contracts finished.
    Contract(&'static [&'static str]),
    /// This faction's relation to the PMC at the most (100).
    MaxRelation(&'static str),
    /// This flag of the save's top table true.
    Flag(&'static str),
}

/// The achievements a save can show, with how (the page's line for it).
pub const RULES: [(&str, Rule, &str); 8] = [
    (
        "ACHIEVEMENT_STAND_UP_AND_SHOUT",
        Rule::MaxRelation("All"),
        "Allied Nations' standing with the PMC at its most",
    ),
    (
        "ACHIEVEMENT_LONGING_FOR_FIRE",
        Rule::MaxRelation("Chi"),
        "China's standing with the PMC at its most",
    ),
    (
        "ACHIEVEMENT_FOREVER_FREE",
        Rule::MaxRelation("Gur"),
        "UPLA's standing with the PMC at its most",
    ),
    (
        "ACHIEVEMENT_DIRTY_DEEDS",
        Rule::MaxRelation("Oil"),
        "Universal Petroleum's standing with the PMC at its most",
    ),
    (
        "ACHIEVEMENT_ISLAND_DOMINATION",
        Rule::MaxRelation("Pir"),
        "The Pirates' standing with the PMC at its most",
    ),
    (
        "ACHIEVEMENT_SCHOOLS_OUT",
        Rule::Contract(&["VzaCon001"]),
        "Earned on the way through the first contract",
    ),
    (
        "ACHIEVEMENT_GONE_SHOOTIN",
        Rule::Contract(&[
            "PmcCon018",
            "PmcCon031",
            "PmcCon032",
            "PmcCon033",
            "PmcCon034",
        ]),
        "Earned finishing one of the PMC's own contracts",
    ),
    (
        "ACHIEVEMENT_RAGE",
        Rule::Flag("bResourceCountersEnabled"),
        "Earned as the resource counters come on",
    ),
];

/// Why the others are not tracked, where it is known.
/// The counted ones (`required` above 1, and the two counted by
/// `Pg.AchievementAddCount` outside the list's counts) keep their counts in
/// the profile.
pub fn untracked_reason(id: &str, required: u32) -> &'static str {
    match id {
        _ if required > 1 => "Not tracked: counted in the player's profile, not in a save",
        "ACHIEVEMENT_HIGHWAY_TO_HELL" | "ACHIEVEMENT_HEAVY_METAL_THUNDER" => {
            "Not tracked: counted in the player's profile, not in a save"
        }
        "ACHIEVEMENT_JUSTICE_FOR_ALL"
        | "ACHIEVEMENT_BALLS_TO_THE_WALL"
        | "ACHIEVEMENT_WHEELS_OF_STEEL"
        | "ACHIEVEMENT_RIDE_DRAGON"
        | "ACHIEVEMENT_BURN_THE_SKY"
        | "ACHIEVEMENT_DIGITAL_MAN" => "Not tracked: granted on a condition the save does not keep",
        _ => "Not tracked: a save does not show it",
    }
}

/// An achievement as one save shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Earned,
    /// Tracked, and not shown earned by this save (it may be, elsewhere).
    NotYet,
    NotTracked,
}

/// The rule for `id`, if a save can show it.
pub fn rule(id: &str) -> Option<(Rule, &'static str)> {
    RULES
        .iter()
        .find(|(r, ..)| *r == id)
        .map(|&(_, rule, how)| (rule, how))
}

/// Whether `save` (the scripts' save text, parsed) shows `rule` met.
pub fn met(rule: Rule, save: &Value) -> bool {
    match rule {
        Rule::Contract(ids) => save
            .get("tFlowData")
            .and_then(|f| f.get("tMyFlowData"))
            .and_then(Value::table)
            .is_some_and(|t| ids.iter().any(|id| t.contains_key(*id))),
        Rule::MaxRelation(f) => save
            .get("tFactionData")
            .and_then(|d| d.get("tRelations"))
            .and_then(|r| r.get(f))
            .and_then(|r| r.get("Pmc"))
            .and_then(Value::number)
            .is_some_and(|n| n >= 100.0),
        Rule::Flag(k) => save.get(k) == Some(&Value::Bool(true)),
    }
}

/// The ids a save shows earned.
pub fn earned(save: &Value) -> Vec<&'static str> {
    RULES
        .iter()
        .filter(|(_, r, _)| met(*r, save))
        .map(|(id, ..)| *id)
        .collect()
}

/// `id`'s state given what a save shows earned (`None`: no save, or its
/// save data could not be read).
pub fn state(id: &str, earned: Option<&[&str]>) -> State {
    match (rule(id), earned) {
        (None, _) | (_, None) => State::NotTracked,
        (Some(_), Some(e)) if e.contains(&id) => State::Earned,
        _ => State::NotYet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save_stats::parse;

    fn function(code: Vec<u32>, constants: Vec<Constant>) -> Chunk {
        Chunk {
            version: m2_assets::lua::Version::Lua51,
            big_endian: true,
            main: Function {
                source: None,
                line_defined: 0,
                last_line_defined: 0,
                upvalue_count: 0,
                param_count: 0,
                is_vararg: 2,
                max_stack: 4,
                code,
                constants,
                functions: Vec::new(),
                lines: Vec::new(),
                locals: Vec::new(),
                upvalues: Vec::new(),
            },
        }
    }

    /// `SETTABLE a RK(b) RK(c)`.
    fn settable(a: u32, b: u32, c: u32) -> u32 {
        u32::from(OP_SETTABLE) | (a << 6) | (c << 14) | (b << 23)
    }

    #[test]
    fn the_list_is_read_from_the_scripts_table() {
        let k = |s: &str| Constant::String(s.to_owned());
        // Constants: "sName", two ids, "nRequiredCount", 20, a stray.
        let constants = vec![
            k("sName"),
            k("ACHIEVEMENT_WILD_ONE"),
            k("nRequiredCount"),
            Constant::Number(20.0),
            k("ACHIEVEMENT_PLAY_COOP"),
            k("tSupportFilter"),
        ];
        let c = |n: u32| RK_CONSTANT + n;
        let code = vec![
            settable(1, c(0), c(1)),
            settable(1, c(2), c(3)),
            settable(2, c(0), c(4)),
            settable(2, c(2), 1), // a register: no count
            settable(0, c(5), 3),
        ];
        let list = read_list(&function(code, constants));
        assert_eq!(
            list,
            [
                Entry {
                    id: "ACHIEVEMENT_WILD_ONE".into(),
                    required: 20
                },
                Entry {
                    id: "ACHIEVEMENT_PLAY_COOP".into(),
                    required: 1
                },
            ]
        );
        let sp = single_player(list);
        assert_eq!(sp.len(), 1);
        assert_eq!(sp[0].title(), "Wild One");
    }

    #[test]
    fn titles_are_the_ids_words() {
        assert_eq!(
            title("ACHIEVEMENT_NO_MORE_MR_NICE_GUY"),
            "No More Mr Nice Guy"
        );
        assert_eq!(title("ACHIEVEMENT_HELLOHURRAY"), "Hellohurray");
        assert_eq!(title("PLAIN"), "Plain");
    }

    #[test]
    fn the_script_is_mrx_achievements() {
        assert_eq!(m2_assets::name_hash("MrxAchievements"), SCRIPT);
    }

    #[test]
    fn a_save_shows_what_its_data_keeps() {
        let save = parse(
            r#"{["bResourceCountersEnabled"]=true,
               ["tFlowData"]={["tMyFlowData"]={["PmcCon032"]=true,},},
               ["tFactionData"]={["tRelations"]={["All"]={["Pmc"]=100,},["Chi"]={["Pmc"]=99,},},},}"#,
        )
        .unwrap();
        let e = earned(&save);
        assert_eq!(
            e,
            [
                "ACHIEVEMENT_STAND_UP_AND_SHOUT",
                "ACHIEVEMENT_GONE_SHOOTIN",
                "ACHIEVEMENT_RAGE"
            ]
        );
        assert_eq!(state("ACHIEVEMENT_RAGE", Some(&e)), State::Earned);
        assert_eq!(
            state("ACHIEVEMENT_LONGING_FOR_FIRE", Some(&e)),
            State::NotYet
        );
        assert_eq!(state("ACHIEVEMENT_SCHOOLS_OUT", Some(&e)), State::NotYet);
        assert_eq!(
            state("ACHIEVEMENT_MILLIONAIRE", Some(&e)),
            State::NotTracked
        );
        // No save data: nothing can be shown.
        assert_eq!(state("ACHIEVEMENT_RAGE", None), State::NotTracked);
        // No flag, or false, is not earned.
        let bare = parse(r#"{["bResourceCountersEnabled"]=false,}"#).unwrap();
        assert!(earned(&bare).is_empty());
    }

    #[test]
    fn every_rule_is_single_player() {
        for (id, ..) in RULES {
            assert!(!MULTIPLAYER.contains(&id), "{id}");
            assert!(id.starts_with(PREFIX));
        }
        assert!(untracked_reason("ACHIEVEMENT_HIGHWAY_TO_HELL", 1).contains("profile"));
        assert!(untracked_reason("ACHIEVEMENT_HOLY_SMOKE", 50).contains("profile"));
        assert!(untracked_reason("ACHIEVEMENT_MILLIONAIRE", 1).contains("does not show"));
    }
}
