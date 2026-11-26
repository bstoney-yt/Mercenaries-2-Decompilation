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
//! - Multiplayer, left out: `ACHIEVEMENT_PLAY_COOP`; `ACHIEVEMENT_DAMAGE_INC`,
//!   which `MrxAchievements.ProcessSupportEvent` counts only while
//!   `Net.IsActive()` (co-op), in `tSupportUsedInCoop`; and, **from the
//!   public lists** (co-op or online in their descriptions),
//!   `WHEELS_OF_STEEL` (its grant goes to the race's `_uWinner`),
//!   `NOTHIN_BUT_GOODTIME` and `EVERYBODY_WANTSSOME`.
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
//! **From public achievement lists** ([`OFFICIAL`]): the 360's titles
//! and gamerscore, matched to the scripts' ids by hand (the ids are close
//! to the titles: `BURNOUT` is Eat the Heat, `HELLOHURRAY` Hello, Hooray).
//! Sources: worthplaying.com's list of 2008-08-09 (the non-secret ones)
//! and vandal.elespanol.com's full list (all 40, 1000G), which agree on
//! every title and score both have. The two faction lists also agree with
//! the scripts' `sMaxRelationAchievement` table. 40 achievements against
//! the scripts' 41 ids: `FIND_ALL_LANDMARKS` and `TECHNO_VIKING` are left
//! unmatched (one is likely Aces High, 50G, "capture every objective"; the
//! other may have been cut), and keep their id-based titles.
//!
//! **Invented**: each line's wording of what earns it ([`Official::about`]
//! and [`RULES`]), and the id-based titles of the two unmatched ids.

use m2_assets::lua::{Chunk, Constant, Function, Instruction};

use crate::save_stats::Value;

/// `MrxAchievements`' script, by its name hash.
pub const SCRIPT: u32 = 0x0fcc_e0ec;

/// The ids' prefix.
const PREFIX: &str = "ACHIEVEMENT_";

/// The multiplayer ones, left off the page.
pub const MULTIPLAYER: [&str; 5] = [
    "ACHIEVEMENT_PLAY_COOP",
    "ACHIEVEMENT_DAMAGE_INC",
    "ACHIEVEMENT_WHEELS_OF_STEEL",
    "ACHIEVEMENT_NOTHIN_BUT_GOODTIME",
    "ACHIEVEMENT_EVERYBODY_WANTSSOME",
];

/// One achievement of the scripts' list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    /// `nRequiredCount`: how many times its event must happen.
    pub required: u32,
}

impl Entry {
    /// The 360's title, else the id's words, capitalised
    /// (`ACHIEVEMENT_FOO_BAR` is "Foo Bar").
    pub fn title(&self) -> String {
        official(&self.id).map_or_else(|| title(&self.id), |o| o.title.to_owned())
    }
}

/// An achievement's name and gamerscore as the 360 shows them, with a line
/// of what earns it in the launcher's own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Official {
    pub id: &'static str,
    pub title: &'static str,
    pub score: u32,
    /// What earns it (the launcher's wording, from the public lists and
    /// what the scripts check).
    pub about: &'static str,
}

const fn o(id: &'static str, title: &'static str, score: u32, about: &'static str) -> Official {
    Official {
        id,
        title,
        score,
        about,
    }
}

/// The 360's titles and gamerscore for the scripts' ids (**from public
/// achievement lists**, matched to the ids by hand: see the module docs).
/// The ids not here (`FIND_ALL_LANDMARKS`, `TECHNO_VIKING`) could not be
/// matched for sure and keep their id-based titles.
pub const OFFICIAL: [Official; 39] = [
    o(
        "ACHIEVEMENT_MILLIONAIRE",
        "Tattooed Millionaire",
        25,
        "Make your first million",
    ),
    o(
        "ACHIEVEMENT_PLAY_COOP",
        "Partners in Crime",
        50,
        "Play co-op",
    ),
    o(
        "ACHIEVEMENT_PIPELINE",
        "Pipeline",
        20,
        "Take every outpost for one faction",
    ),
    o(
        "ACHIEVEMENT_BURNOUT",
        "Eat the Heat",
        10,
        "Spin the wheels on the spot",
    ),
    o(
        "ACHIEVEMENT_SCHOOLS_OUT",
        "School's Out",
        20,
        "Get through the opening contract's training",
    ),
    o(
        "ACHIEVEMENT_RAGE",
        "The Rage",
        20,
        "Finish the contract that frees Carmona",
    ),
    o(
        "ACHIEVEMENT_HELLOHURRAY",
        "Hello, Hooray",
        50,
        "Set up the PMC",
    ),
    o(
        "ACHIEVEMENT_WILD_ONE",
        "Wild One",
        20,
        "Bring Ewan on board",
    ),
    o(
        "ACHIEVEMENT_RIDE_DRAGON",
        "Ride the Dragon",
        10,
        "Take the tank from the village",
    ),
    o(
        "ACHIEVEMENT_ARMAGGEDON",
        "Armaggedon It",
        20,
        "Bring down 20 buildings",
    ),
    o(
        "ACHIEVEMENT_SHOOTTHRILL",
        "Shoot to Thrill",
        20,
        "Score 25 headshots",
    ),
    o(
        "ACHIEVEMENT_DAMAGE_INC",
        "Damage, Inc.",
        50,
        "Call 20 air strikes in co-op",
    ),
    o(
        "ACHIEVEMENT_NO_COMPROMISE",
        "No Compromise, No Regrets",
        20,
        "Take part in the fight for Caracas",
    ),
    o(
        "ACHIEVEMENT_RUNNING_WITH_DEVIL",
        "Runnin' with the Devil",
        20,
        "Deal with Blanco",
    ),
    o(
        "ACHIEVEMENT_BETTER_RUN",
        "You Better Run",
        20,
        "Deal with Carmona",
    ),
    o(
        "ACHIEVEMENT_ANALOG_KID",
        "The Analog Kid",
        20,
        "Bring Misha on board",
    ),
    o(
        "ACHIEVEMENT_OIL_AND_GAZ",
        "Oil and Gasoline",
        20,
        "Bring Eva on board",
    ),
    o(
        "ACHIEVEMENT_HERO_AND_MADMAN",
        "The Hero and the Madman",
        50,
        "Finish the story",
    ),
    o(
        "ACHIEVEMENT_HAIL_AND_KILL",
        "Hail and Kill",
        20,
        "Wreck 50 things in melee",
    ),
    o(
        "ACHIEVEMENT_HOLY_SMOKE",
        "Holy Smoke",
        20,
        "Wreck 50 things with grenades",
    ),
    o(
        "ACHIEVEMENT_LITTLE_SAVAGE",
        "Little Savage",
        20,
        "Wreck 50 things with rockets",
    ),
    o(
        "ACHIEVEMENT_BALLS_TO_THE_WALL",
        "Balls to the Wall",
        20,
        "Beat the winch challenge",
    ),
    o(
        "ACHIEVEMENT_GONE_SHOOTIN",
        "Gone Shootin'",
        20,
        "Finish one of Fiona's hunting contracts",
    ),
    o(
        "ACHIEVEMENT_HEAVY_METAL_THUNDER",
        "Heavy Metal Thunder",
        10,
        "Steal 10 vehicles",
    ),
    o(
        "ACHIEVEMENT_QUICK_OR_DEAD",
        "Be Quick or Be Dead",
        10,
        "Snipe 50 drivers",
    ),
    o(
        "ACHIEVEMENT_STAND_UP_AND_SHOUT",
        "Stand Up and Shout",
        10,
        "Allied Nations' standing at its best",
    ),
    o(
        "ACHIEVEMENT_FOREVER_FREE",
        "Forever Free",
        10,
        "UPLA's standing at its best",
    ),
    o(
        "ACHIEVEMENT_ISLAND_DOMINATION",
        "Island of Domination",
        10,
        "The Pirates' standing at its best",
    ),
    o(
        "ACHIEVEMENT_LONGING_FOR_FIRE",
        "Longing for Fire",
        10,
        "China's standing at its best",
    ),
    o(
        "ACHIEVEMENT_DIRTY_DEEDS",
        "Dirty Deeds",
        10,
        "Universal Petroleum's standing at its best",
    ),
    o(
        "ACHIEVEMENT_NO_MORE_MR_NICE_GUY",
        "No More Mr. Nice Guy",
        10,
        "Have every faction hostile at once",
    ),
    o(
        "ACHIEVEMENT_JUSTICE_FOR_ALL",
        "...And Justice for All",
        50,
        "Verify every high-value target",
    ),
    o(
        "ACHIEVEMENT_DIGITAL_MAN",
        "Digital Man",
        20,
        "Unlock everything in the shops",
    ),
    o(
        "ACHIEVEMENT_BURN_THE_SKY",
        "We'll Burn the Sky",
        50,
        "Open every landing zone",
    ),
    o(
        "ACHIEVEMENT_HIGHWAY_TO_HELL",
        "Highway to Hell",
        10,
        "Finish three different races",
    ),
    o(
        "ACHIEVEMENT_WHEELS_OF_STEEL",
        "Wheels of Steel",
        50,
        "Win a co-op race",
    ),
    o(
        "ACHIEVEMENT_NOTHIN_BUT_GOODTIME",
        "Nothin' But a Good Time",
        50,
        "Wreck 200 things from a gunner's seat in co-op",
    ),
    o(
        "ACHIEVEMENT_EVERYBODY_WANTSSOME",
        "Everybody Wants Some!!",
        50,
        "Play online with someone who has it",
    ),
    o(
        "ACHIEVEMENT_BILLIONAIRE",
        "Billion Dollar Babies",
        25,
        "Make a billion",
    ),
];

/// The 360's name and score for `id`, where it is known.
pub fn official(id: &str) -> Option<&'static Official> {
    OFFICIAL.iter().find(|o| o.id == id)
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
        assert_eq!(
            Entry {
                id: "ACHIEVEMENT_BURNOUT".into(),
                required: 1
            }
            .title(),
            "Eat the Heat"
        );
        assert_eq!(
            Entry {
                id: "ACHIEVEMENT_TECHNO_VIKING".into(),
                required: 1
            }
            .title(),
            "Techno Viking"
        );
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
    fn the_official_list_adds_up() {
        // 39 matched of the 360's 40 (Aces High, 50G, unmatched): 950G.
        assert_eq!(OFFICIAL.iter().map(|o| o.score).sum::<u32>(), 950);
        let mut ids: Vec<_> = OFFICIAL.iter().map(|o| o.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), OFFICIAL.len());
        for m in MULTIPLAYER {
            assert!(official(m).is_some(), "{m}");
        }
        for (id, ..) in RULES {
            assert!(official(id).is_some(), "{id}");
        }
        assert!(official("ACHIEVEMENT_FIND_ALL_LANDMARKS").is_none());
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
