//! The human state table (`ece70371`): for each stance and action a
//! person can be in, what each message moves them to.
//!
//! A person's animation is asked for by a stance and an action
//! ([`crate::animation_table`]). Which action comes next is not the
//! program's to say: code posts a message to the person (`JumpMoving` when
//! the jump is pressed on the move, `JumpInAir` when the physics leaves
//! the ground, `Complete` when the action's clip ends) and this table
//! gives the stance and action that message leads to from the state they
//! are in. A jump runs through it so: `Upright` `Idle`, on `JumpMoving`,
//! goes to `Upright` `JumpTakeoffMoving`; that, on `JumpInAir`, to
//! `Upright` `Jump`; that, on `Complete`, to `InAir` `Jump`; and that, on
//! `MovingOnGround`, to `Upright` `JumpLandMoving`, whose `Complete`
//! leads back to `Upright` `Idle`.
//!
//! A `UCFX` container, everything little-endian, every name plain text:
//!
//! ```text
//! INFO   u16 (5 in the game's; not read by the loader), u16 stance count
//! stns/  one per stance:
//!   SINF   cstr stance, u16 action count
//!   actn/  one per action:
//!     AINF   cstr action, u16 transition count, then six cstr words:
//!            five settings (`TRUE`, `FALSE`; the first may be
//!            `MOVEONLY`) and a mode (`NONE`, `FULL`, `ON_END`)
//!     TRNS   one per transition: cstr message, cstr stance, cstr action,
//!            u32 (0 in every one of the game's)
//! ```
//!
//! # Provenance
//!
//! Format-from-samples: the one such asset of a legally installed copy
//! (in `vz.wad`'s `resident` block: 17 stances, 394 actions, 8,744
//! transitions), which parses whole with these rules, every count
//! matching the sections that follow it. No action lists a message twice.
//! No bytes from the archives are committed.
//!
//! From the 360 code: the loader (stances 0x82422AD8, a stance's actions
//! 0x82422DE0, an action 0x82423128). It reads the fields in the order
//! above and keeps only the hashes of the names ([`crate::name_hash`]),
//! which is why the names are found nowhere in the program. Of an
//! action's words it keeps, at these offsets of its record:
//!
//! - +0x1c: the first word is `TRUE`;
//! - +0x21: the first word is `TRUE` or `MOVEONLY`;
//! - +0x1d, +0x1e, +0x1f, +0x20: the second to fifth words are `TRUE`;
//! - +0x24: the sixth word's hash.
//!
//! Any other spelling counts as not `TRUE` (the game's has one `FASLE` and
//! one action with every word empty). It then sorts each action's
//! transitions (0x824239C8) and each stance's actions (0x82423B20); a
//! lookup here is by name, which finds the same one.
//!
//! **Not traced**: what the settings do. They are named by their offsets.
//! Read from which actions set them (inferred, not from code): +0x1c and
//! +0x21 go with actions she can move in (`Idle`, `Jump`,
//! `JumpTakeoffMoving`, the reloads; not `JumpTakeoffStationary`), +0x1d
//! with ones she can use a weapon in, +0x1e and +0x1f are off for the
//! moves that take the whole body over (dives, vaults, staggers, fall
//! impacts), +0x20 is on for a few she is held in (grapples, knockdowns,
//! melee counters), and the mode is `FULL` for the deaths and knockdowns
//! and `ON_END` for `DieAnimatedRagdoll`, so it reads as when the ragdoll
//! takes over. The animation driver (0x82420A00) compares the mode with
//! `ON_END`'s hash.

use crate::bytes::{OutOfBounds, Reader};
use crate::name_hash;
use crate::ucfx::{Ucfx, UcfxError};

/// The asset's name hash, and its type's.
pub const HUMAN_STATES: u32 = 0xece7_0371;
pub const HUMAN_STATES_TYPE: u32 = 0xece7_0371;

const INFO: [u8; 4] = *b"INFO";
const STNS: [u8; 4] = *b"stns";
const SINF: [u8; 4] = *b"SINF";
const ACTN: [u8; 4] = *b"actn";
const AINF: [u8; 4] = *b"AINF";
const TRNS: [u8; 4] = *b"TRNS";

/// Errors from parsing a human state table.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HumanStatesError {
    #[error(transparent)]
    Ucfx(#[from] UcfxError),
    #[error("section truncated: {len} bytes at {offset}")]
    Truncated { offset: usize, len: usize },
    #[error("no INFO section")]
    NoInfo,
    #[error("a name is not UTF-8")]
    Name,
    #[error("{tag} section outside its place in the tree")]
    Stray { tag: String },
    #[error("{what} count {stated} but {found} found")]
    Count {
        what: &'static str,
        stated: usize,
        found: usize,
    },
}

impl From<OutOfBounds> for HumanStatesError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// What a message leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub message: String,
    pub stance: String,
    pub action: String,
    /// The word after the names; 0 in every one of the game's.
    pub unk_0x0c: u32,
}

/// One action of a stance: its settings as the loader keeps them (named
/// by the offsets of its record), its words as written, and where each
/// message leads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Action {
    pub name: String,
    /// The first word is `TRUE`.
    pub flag_0x1c: bool,
    /// The second to fifth words are `TRUE`.
    pub flag_0x1d: bool,
    pub flag_0x1e: bool,
    pub flag_0x1f: bool,
    pub flag_0x20: bool,
    /// The first word is `TRUE` or `MOVEONLY`.
    pub flag_0x21: bool,
    /// The sixth word's hash ([`MODE_NONE`], [`MODE_FULL`], [`MODE_ON_END`]).
    pub mode_0x24: u32,
    /// The six words as the asset spells them.
    pub words: [String; 6],
    pub transitions: Vec<Transition>,
}

/// The sixth word's hashes in the game's table.
pub const MODE_NONE: u32 = 0x4ff9_f863;
pub const MODE_FULL: u32 = 0x4294_eba2;
pub const MODE_ON_END: u32 = 0x7ac5_01f8;

const TRUE: u32 = 0x1e52_08dd;
const MOVE_ONLY: u32 = 0xd955_094c;

impl Action {
    /// Where `message` leads from this action, names compared by hash as
    /// the game does (so in any case).
    pub fn on(&self, message: &str) -> Option<&Transition> {
        let h = name_hash(message);
        self.transitions.iter().find(|t| name_hash(&t.message) == h)
    }
}

/// A stance and its actions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stance {
    pub name: String,
    pub actions: Vec<Action>,
}

impl Stance {
    pub fn action(&self, name: &str) -> Option<&Action> {
        let h = name_hash(name);
        self.actions.iter().find(|a| name_hash(&a.name) == h)
    }
}

/// A parsed human state table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HumanStates {
    /// `INFO`'s first word (5 in the game's; the loader skips it).
    pub unk_0x00: u16,
    pub stances: Vec<Stance>,
}

fn text(bytes: &[u8]) -> Result<String, HumanStatesError> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| HumanStatesError::Name)
}

impl HumanStates {
    /// Parses a human state table asset.
    pub fn parse(asset: &[u8]) -> Result<Self, HumanStatesError> {
        let u = Ucfx::parse(asset)?;
        let mut info = Reader::at(u.top_level(INFO).ok_or(HumanStatesError::NoInfo)?, 0);
        let unk_0x00 = info.u16()?;
        let stated = usize::from(info.u16()?);
        let mut stances: Vec<Stance> = Vec::new();
        // The counts each SINF and AINF state, checked once all is read.
        let mut actions_stated: Vec<usize> = Vec::new();
        let mut transitions_stated: Vec<Vec<usize>> = Vec::new();
        let stray = |tag: [u8; 4]| HumanStatesError::Stray {
            tag: String::from_utf8_lossy(&tag).into_owned(),
        };
        for s in u.sections() {
            match (s.fourcc, s.depth, s.data) {
                (STNS, 0, _) => {
                    stances.push(Stance::default());
                    actions_stated.push(0);
                    transitions_stated.push(Vec::new());
                }
                (SINF, 1, Some(data)) => {
                    let stance = stances.last_mut().ok_or_else(|| stray(SINF))?;
                    let mut r = Reader::at(data, 0);
                    stance.name = text(r.cstr()?)?;
                    *actions_stated.last_mut().ok_or_else(|| stray(SINF))? = usize::from(r.u16()?);
                }
                (ACTN, 1, _) => {
                    let stance = stances.last_mut().ok_or_else(|| stray(ACTN))?;
                    stance.actions.push(Action::default());
                    transitions_stated
                        .last_mut()
                        .ok_or_else(|| stray(ACTN))?
                        .push(0);
                }
                (AINF, 2, Some(data)) => {
                    let action = stances
                        .last_mut()
                        .and_then(|s| s.actions.last_mut())
                        .ok_or_else(|| stray(AINF))?;
                    let mut r = Reader::at(data, 0);
                    action.name = text(r.cstr()?)?;
                    *transitions_stated
                        .last_mut()
                        .and_then(|t| t.last_mut())
                        .ok_or_else(|| stray(AINF))? = usize::from(r.u16()?);
                    for word in &mut action.words {
                        *word = text(r.cstr()?)?;
                    }
                    let is = |i: usize, h: u32| name_hash(&action.words[i]) == h;
                    action.flag_0x1c = is(0, TRUE);
                    action.flag_0x21 = is(0, TRUE) || is(0, MOVE_ONLY);
                    action.flag_0x1d = is(1, TRUE);
                    action.flag_0x1e = is(2, TRUE);
                    action.flag_0x1f = is(3, TRUE);
                    action.flag_0x20 = is(4, TRUE);
                    action.mode_0x24 = name_hash(&action.words[5]);
                }
                (TRNS, 2, Some(data)) => {
                    let action = stances
                        .last_mut()
                        .and_then(|s| s.actions.last_mut())
                        .ok_or_else(|| stray(TRNS))?;
                    let mut r = Reader::at(data, 0);
                    action.transitions.push(Transition {
                        message: text(r.cstr()?)?,
                        stance: text(r.cstr()?)?,
                        action: text(r.cstr()?)?,
                        unk_0x0c: r.u32()?,
                    });
                }
                (INFO, 0, _) => {}
                (tag, ..) => return Err(stray(tag)),
            }
        }
        let count = |what, stated: usize, found: usize| {
            if stated == found {
                Ok(())
            } else {
                Err(HumanStatesError::Count {
                    what,
                    stated,
                    found,
                })
            }
        };
        count("stance", stated, stances.len())?;
        for (i, stance) in stances.iter().enumerate() {
            count("action", actions_stated[i], stance.actions.len())?;
            for (j, action) in stance.actions.iter().enumerate() {
                count(
                    "transition",
                    transitions_stated[i][j],
                    action.transitions.len(),
                )?;
            }
        }
        Ok(Self { unk_0x00, stances })
    }

    pub fn stance(&self, name: &str) -> Option<&Stance> {
        let h = name_hash(name);
        self.stances.iter().find(|s| name_hash(&s.name) == h)
    }

    /// Where `message` leads a person in `stance` doing `action`: the new
    /// stance and action. `None` when the state is not in the table or
    /// has no transition for the message (the message is then ignored).
    pub fn next(&self, stance: &str, action: &str, message: &str) -> Option<&Transition> {
        self.stance(stance)?.action(action)?.on(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn cstrs(words: &[&str]) -> Vec<u8> {
        words
            .iter()
            .flat_map(|w| w.bytes().chain(std::iter::once(0)))
            .collect()
    }

    fn info(stances: u16) -> UcfxNode {
        let mut d = 5u16.to_le_bytes().to_vec();
        d.extend_from_slice(&stances.to_le_bytes());
        UcfxNode::Leaf(INFO, d)
    }

    fn action(name: &str, words: [&str; 6], transitions: &[[&str; 3]]) -> UcfxNode {
        let mut ainf = cstrs(&[name]);
        ainf.extend_from_slice(&(transitions.len() as u16).to_le_bytes());
        ainf.extend(cstrs(&words));
        let mut nodes = vec![UcfxNode::Leaf(AINF, ainf)];
        for t in transitions {
            let mut d = cstrs(t);
            d.extend_from_slice(&0u32.to_le_bytes());
            nodes.push(UcfxNode::Leaf(TRNS, d));
        }
        UcfxNode::Container(ACTN, nodes)
    }

    fn stance(name: &str, actions: Vec<UcfxNode>) -> UcfxNode {
        let mut sinf = cstrs(&[name]);
        sinf.extend_from_slice(&(actions.len() as u16).to_le_bytes());
        let mut nodes = vec![UcfxNode::Leaf(SINF, sinf)];
        nodes.extend(actions);
        UcfxNode::Container(STNS, nodes)
    }

    const PLAIN: [&str; 6] = ["TRUE", "FALSE", "FALSE", "TRUE", "FALSE", "NONE"];

    /// A table laid out as the game's, with the rows a jump runs through
    /// (the game's own words).
    fn jump() -> Vec<u8> {
        ucfx(&[
            info(2),
            stance(
                "Upright",
                vec![
                    action(
                        "Idle",
                        ["TRUE", "TRUE", "TRUE", "TRUE", "FALSE", "NONE"],
                        &[
                            ["JumpMoving", "Upright", "JumpTakeoffMoving"],
                            ["JumpStationary", "Upright", "JumpTakeoffStationary"],
                            ["Falling", "InAir", "Fall"],
                        ],
                    ),
                    action(
                        "JumpTakeoffMoving",
                        PLAIN,
                        &[
                            ["Complete", "Upright", "Idle"],
                            ["JumpInAir", "Upright", "Jump"],
                        ],
                    ),
                    action(
                        "Jump",
                        PLAIN,
                        &[
                            ["Complete", "InAir", "Jump"],
                            ["OnGround", "Upright", "JumpLandStationary"],
                            ["MovingOnGround", "Upright", "JumpLandMoving"],
                        ],
                    ),
                    action("JumpLandMoving", PLAIN, &[["Complete", "Upright", "Idle"]]),
                    action(
                        "DieAnimatedRagdoll",
                        ["FALSE", "FALSE", "FALSE", "TRUE", "FASLE", "ON_END"],
                        &[],
                    ),
                    action("VerifyCamera", [""; 6], &[]),
                ],
            ),
            stance(
                "InAir",
                vec![
                    action(
                        "Jump",
                        PLAIN,
                        &[
                            ["OnGround", "Upright", "JumpLandStationary"],
                            ["MovingOnGround", "Upright", "JumpLandMoving"],
                        ],
                    ),
                    action(
                        "Grapple",
                        ["MOVEONLY", "TRUE", "FALSE", "TRUE", "TRUE", "NONE"],
                        &[],
                    ),
                ],
            ),
        ])
    }

    #[test]
    fn a_jump_runs_through_the_table() {
        let t = HumanStates::parse(&jump()).unwrap();
        assert_eq!(t.unk_0x00, 5);
        assert_eq!(t.stances.len(), 2);
        let mut at = ("Upright".to_owned(), "Idle".to_owned());
        let mut been = Vec::new();
        for message in [
            "JumpMoving",
            "JumpInAir",
            "Complete",
            "MovingOnGround",
            "Complete",
        ] {
            let next = t.next(&at.0, &at.1, message).unwrap();
            assert_eq!(next.unk_0x0c, 0);
            at = (next.stance.clone(), next.action.clone());
            been.push(format!("{}.{}", at.0, at.1));
        }
        assert_eq!(
            been,
            [
                "Upright.JumpTakeoffMoving",
                "Upright.Jump",
                "InAir.Jump",
                "Upright.JumpLandMoving",
                "Upright.Idle"
            ]
        );
    }

    #[test]
    fn names_match_by_hash_and_an_unlisted_message_leads_nowhere() {
        let t = HumanStates::parse(&jump()).unwrap();
        // In any case, as the game's hash is.
        assert_eq!(
            t.next("upright", "IDLE", "jumpstationary")
                .map(|n| n.action.as_str()),
            Some("JumpTakeoffStationary")
        );
        assert_eq!(t.next("Upright", "Idle", "OnGround"), None);
        assert_eq!(t.next("Upright", "Swim", "Complete"), None);
        assert_eq!(t.next("Prone", "Idle", "Complete"), None);
    }

    #[test]
    fn an_actions_words_are_kept_as_the_loader_keeps_them() {
        let t = HumanStates::parse(&jump()).unwrap();
        let of = |s: &str, a: &str| t.stance(s).unwrap().action(a).unwrap().clone();
        let idle = of("Upright", "Idle");
        assert!(idle.flag_0x1c && idle.flag_0x21 && idle.flag_0x1d);
        assert!(idle.flag_0x1e && idle.flag_0x1f && !idle.flag_0x20);
        assert_eq!(idle.mode_0x24, MODE_NONE);
        // `MOVEONLY` sets +0x21 alone.
        let grapple = of("InAir", "Grapple");
        assert!(!grapple.flag_0x1c && grapple.flag_0x21 && grapple.flag_0x20);
        // A misspelt `FALSE` is not `TRUE`; the mode is the word's hash.
        let die = of("Upright", "DieAnimatedRagdoll");
        assert!(!die.flag_0x20 && die.flag_0x1f);
        assert_eq!(die.mode_0x24, MODE_ON_END);
        assert_eq!(die.words[4], "FASLE");
        // Every word empty: nothing set, and the empty word's hash.
        let blank = of("Upright", "VerifyCamera");
        assert!(!blank.flag_0x1c && !blank.flag_0x21 && !blank.flag_0x1f);
        assert_eq!(blank.mode_0x24, name_hash(""));
    }

    #[test]
    fn the_words_hash_to_the_loaders_constants() {
        assert_eq!(name_hash("TRUE"), TRUE);
        assert_eq!(name_hash("MOVEONLY"), MOVE_ONLY);
        assert_eq!(name_hash("NONE"), MODE_NONE);
        assert_eq!(name_hash("FULL"), MODE_FULL);
        assert_eq!(name_hash("ON_END"), MODE_ON_END);
    }

    #[test]
    fn a_count_that_disagrees_with_the_sections_is_refused() {
        let short = ucfx(&[info(2), stance("Upright", vec![])]);
        assert_eq!(
            HumanStates::parse(&short),
            Err(HumanStatesError::Count {
                what: "stance",
                stated: 2,
                found: 1
            })
        );
        // An AINF stating two transitions with one following.
        let mut ainf = cstrs(&["Idle"]);
        ainf.extend_from_slice(&2u16.to_le_bytes());
        ainf.extend(cstrs(&PLAIN));
        let mut trns = cstrs(&["Complete", "Upright", "Idle"]);
        trns.extend_from_slice(&0u32.to_le_bytes());
        let mut sinf = cstrs(&["Upright"]);
        sinf.extend_from_slice(&1u16.to_le_bytes());
        let bad = ucfx(&[
            info(1),
            UcfxNode::Container(
                STNS,
                vec![
                    UcfxNode::Leaf(SINF, sinf),
                    UcfxNode::Container(
                        ACTN,
                        vec![UcfxNode::Leaf(AINF, ainf), UcfxNode::Leaf(TRNS, trns)],
                    ),
                ],
            ),
        ]);
        assert_eq!(
            HumanStates::parse(&bad),
            Err(HumanStatesError::Count {
                what: "transition",
                stated: 2,
                found: 1
            })
        );
    }

    #[test]
    fn truncated_and_missing_sections_are_errors() {
        assert_eq!(
            HumanStates::parse(&ucfx(&[stance("Upright", vec![])])),
            Err(HumanStatesError::NoInfo)
        );
        // A TRNS cut off inside its last word.
        let mut ainf = cstrs(&["Idle"]);
        ainf.extend_from_slice(&1u16.to_le_bytes());
        ainf.extend(cstrs(&PLAIN));
        let mut sinf = cstrs(&["Upright"]);
        sinf.extend_from_slice(&1u16.to_le_bytes());
        let cut = ucfx(&[
            info(1),
            UcfxNode::Container(
                STNS,
                vec![
                    UcfxNode::Leaf(SINF, sinf),
                    UcfxNode::Container(
                        ACTN,
                        vec![
                            UcfxNode::Leaf(AINF, ainf),
                            UcfxNode::Leaf(TRNS, cstrs(&["Complete", "Upright", "Idle"])),
                        ],
                    ),
                ],
            ),
        ]);
        assert!(matches!(
            HumanStates::parse(&cut),
            Err(HumanStatesError::Truncated { .. })
        ));
    }
}
