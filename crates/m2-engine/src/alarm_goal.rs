//! A soldier running to raise an alarm: the AI's `triggeralarm` goal and
//! its task, `humanTriggerAlarm` ([`crate`]'s side; the tower's flag and
//! script are `m2_game::alarm`'s).
//!
//! From the 360 code:
//!
//! - Each think, an AI on foot rates every object it knows of through ten
//!   proposers (the table at 82B77DE8, run by 822DDF50 for the bits of its
//!   mask, 0x3FF on foot and 0x39 otherwise, 822D7D50). The tenth,
//!   822E1DD0, proposes going to raise an alarm. It does when the object
//!   is an alarm whose flag is off (the store at 83657E5C), of the
//!   soldier's own faction, no further than 50 m ([`RANGE`], 820B7FB8),
//!   while the soldier has someone to fight, isn't held to a point (an
//!   anchor of radius 0 to 1e-4, 822D66A8), and the alarm isn't another
//!   soldier's to raise ([`may_claim`]).
//! - The goal is kind 0x18 (`"triggeralarm"`, the goal names at 82B77E38)
//!   at priority 9.9 ([`PRIORITY`], 820F98AC), above a script's `medPri`
//!   (3) and below its `hiPri` (14).
//! - One soldier at a time raises a given alarm: the claim table at
//!   83656DB4 holds, for each object, who claimed it and with what score;
//!   another may take it over only by a score at least 1.1 times the
//!   holder's ([`HANDOVER`], 82DE8C20; 8229EB00). The score is 50 less
//!   the distance ([`score`]), renewed each think (822C0900).
//! - The task (82B7AA40: start 822B5050, think 822B4D70, still wanted
//!   822B5230) takes the tower's four hardpoints ([`HARDPOINTS`], nodes of
//!   the tower model), nearest first ([`by_distance`]), and runs to the
//!   first. Within 0.5 m of it over the ground ([`REACH`], 820D9EDC,
//!   [`reached`]) the soldier plays his use clip (0x29EFE0B5) and, the flag
//!   still off, posts the alarm on (`m2_game::alarm::ON`). A hardpoint the
//!   model lacks is skipped. The task ends when the flag is on or the
//!   claim is lost.
//!
//! Inferred: that the float the proposer reads from the knowledge record
//! (+0x5c, copied with a vector from the AI's sensing record, 822D5468) is
//! the distance to the object; that the faction it compares (+0xA8) is
//! the object's `Association`.

/// How far a soldier goes to raise an alarm, metres (820B7FB8).
pub const RANGE: f32 = 50.0;

/// How near a hardpoint he must stand, metres over the ground (820D9EDC).
pub const REACH: f32 = 0.5;

/// How much better a score must be to take an alarm from the soldier
/// holding it (82DE8C20).
pub const HANDOVER: f32 = 1.1;

/// The goal's priority (820F98AC).
pub const PRIORITY: f32 = 9.9;

/// The goal kind (`"triggeralarm"`).
pub const GOAL_KIND: u32 = 0x18;

/// The clip he plays at the tower (822B4D70).
pub const USE_CLIP: u32 = 0x29ef_e0b5;

/// The tower's hardpoints he runs to, by name hash (822B5050; the tower
/// model's nodes 19 to 22).
pub const HARDPOINTS: [u32; 4] = [0x66ac_c70f, 0xdcaf_bf68, 0xc6b1_db5d, 0xbca0_827e];

/// A soldier's score for an alarm `distance` metres off: the nearer, the
/// higher.
pub fn score(distance: f32) -> f32 {
    RANGE - distance
}

/// Whether a soldier scoring `mine` may claim an alarm `held` by another
/// with that score: when nobody holds it, or `mine` is at least
/// [`HANDOVER`] times theirs (8229EB00).
pub fn may_claim(held: Option<f32>, mine: f32) -> bool {
    held.is_none_or(|theirs| theirs * HANDOVER <= mine)
}

/// What a soldier and an alarm are, for whether he goes to raise it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// The alarm's flag is on.
    pub on: bool,
    /// It is of his faction.
    pub same_faction: bool,
    /// He has someone to fight.
    pub fighting: bool,
    /// He is held to a point (an anchor of radius at most 1e-4).
    pub pinned: bool,
    /// How far it is, metres.
    pub distance: f32,
}

/// Whether he goes to raise it, before the claim (822E1DD0).
pub fn proposes(c: &Candidate) -> bool {
    !c.on && c.same_faction && c.fighting && !c.pinned && c.distance <= RANGE
}

/// An anchor radius that holds him to a point (822D66A8: 0 to 1e-4).
pub fn pins(radius: f32) -> bool {
    (0.0..=1e-4).contains(&radius)
}

/// The hardpoints there are, nearest `from` first (the squared distance in
/// all three axes, 822B5050).
pub fn by_distance(from: [f32; 3], hardpoints: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let d2 = |p: &[f32; 3]| {
        let o = [p[0] - from[0], p[1] - from[1], p[2] - from[2]];
        o[1] * o[1] + o[2] * o[2] + o[0] * o[0]
    };
    let mut out = hardpoints.to_vec();
    out.sort_by(|a, b| d2(a).total_cmp(&d2(b)));
    out
}

/// Whether he stands at a hardpoint: within [`REACH`] over the ground.
pub fn reached(at: [f32; 3], hardpoint: [f32; 3]) -> bool {
    let (x, z) = (hardpoint[0] - at[0], hardpoint[2] - at[2]);
    z * z + x * x <= REACH * REACH
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> Candidate {
        Candidate {
            on: false,
            same_faction: true,
            fighting: true,
            pinned: false,
            distance: 30.0,
        }
    }

    #[test]
    fn a_fighting_soldier_of_its_faction_within_50_m_goes() {
        assert!(proposes(&guard()));
        assert!(proposes(&Candidate {
            distance: 50.0,
            ..guard()
        }));
        for c in [
            Candidate {
                distance: 50.5,
                ..guard()
            },
            Candidate { on: true, ..guard() },
            Candidate {
                same_faction: false,
                ..guard()
            },
            Candidate {
                fighting: false,
                ..guard()
            },
            Candidate {
                pinned: true,
                ..guard()
            },
        ] {
            assert!(!proposes(&c), "{c:?}");
        }
    }

    #[test]
    fn a_nearer_soldier_takes_the_alarm_only_by_a_tenth_more() {
        assert_eq!(score(30.0), 20.0);
        assert!(may_claim(None, 0.0));
        // Held at 20 (30 m off): 21 isn't enough, 22 is.
        assert!(!may_claim(Some(20.0), 21.0));
        assert!(may_claim(Some(20.0), 22.0 + 1e-4));
    }

    #[test]
    fn only_an_anchor_of_no_size_pins() {
        assert!(pins(0.0) && pins(1e-4));
        assert!(!pins(-1.0) && !pins(0.5) && !pins(50.0));
    }

    #[test]
    fn the_nearest_hardpoint_first_and_half_a_metre_over_the_ground() {
        let hp = [[3.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        assert_eq!(
            by_distance([0.0; 3], &hp),
            [[1.0, 0.0, 0.0], [0.0, 2.0, 0.0], [3.0, 0.0, 0.0]]
        );
        assert!(reached([0.3, 0.0, 0.3], [0.0, 5.0, 0.0]));
        assert!(!reached([0.4, 0.0, 0.4], [0.0, 0.0, 0.0]));
    }
}
