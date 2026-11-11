//! Whom a fighter goes for: the 360's target choice among all they
//! perceive, the player being one of those. From the 360 code unless a
//! line says otherwise.
//!
//! Each subject a mind perceives has an attitude, its faction's relation
//! (-100 to 100, 0x822F0600) read at 0x822D5948. A table built into the
//! program (0x822EB658, read by 0x82163CE0) turns it into what the mind
//! may do: only below [`ATTACK_BELOW`] may it attack (bit 2, 0x822E0D68).
//! Each subject it may attack is scored (0x822E0988) and offered for the
//! mind's attack slot (0x822C0318): the one in it keeps its place with
//! its own new weight, and a challenger needs a weight above that.
//!
//! The score is what the subject is worth ([`worth`]: how much of a
//! threat it is to the mind, the perception record's +0xC8, [`threat`],
//! plus [`BASE`]; or [`PRIORITY`] for one the scripts made a priority
//! target), times how well the mind can see it (+0x68, [`visible`]) and
//! by the mind's own [`Heat`] (+0x804). Then the distance: within
//! [`NEAR`] (or [`HOLD`] for the one in the slot) a subject counts double,
//! and one nearer than the one in the slot gains [`CLOSER`].
//!
//! Not traced yet: 0x822E0988 also puts 1.2 in place of the worth for
//! vehicles of two kinds (their object's +0x170 at 4 or 5), 9.8 for people
//! of one class (0x823F7B88 at 0x80008240), and lifts the score to
//! `0.2 x + 0.8` for the subject in the mind's +0xC58 order while that
//! order weighs more than 0.3.
//!
//! What a mind holds of a subject is not the relation itself but its own
//! attitude ([`Attitude`], the record's +0xE4), which follows the
//! relation (+0xA0): down at once, back up slowly. Made worse, a mind is
//! hostile at once; made better, it calms over time.

/// How fast an attitude climbs back toward a better relation, per second
/// (0x820BAF0C, 0x822DE240).
pub const CALMS_PER_SECOND: f32 = 0.3;
/// How far an attitude may sit from the relation before it moves
/// (0x820BAEE8, 0x822DE240).
const SLACK: f32 = 1.0;

/// A mind's attitude to a subject it perceives: the perception record's
/// +0xE4, set through 0x822D5BF8.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attitude(pub f32);

impl Attitude {
    /// A new record takes the relation as it is (0x822D5948 with its
    /// last argument set).
    pub fn new(relation: f32) -> Self {
        Self(relation)
    }

    /// One think, `dt` seconds: the record's age step (0x822DE240) moves
    /// the attitude [`CALMS_PER_SECOND`] toward the relation once it is
    /// more than [`SLACK`] off, either way; then the relation is read
    /// again (0x822D5948), and one below the attitude replaces it.
    pub fn think(&mut self, relation: f32, dt: f32) {
        let step = CALMS_PER_SECOND * dt;
        if self.0 > relation + SLACK {
            self.0 -= step;
        } else if self.0 < relation - SLACK {
            self.0 += step;
        }
        if relation < self.0 {
            self.0 = relation;
        }
    }

    /// Whether the mind may attack the subject now ([`attacks`]).
    pub fn attacks(self) -> bool {
        attacks(self.0)
    }
}

/// A relation below this may be attacked (the program's first band,
/// -100 to -33, rule bits 7).
pub const ATTACK_BELOW: f32 = -33.0;
/// Added to a subject's threat before it is scored (0x820BB468).
const BASE: f32 = 0.1;
/// Of the reach a subject can be seen from, where its visibility
/// rating is a half (0x820CAF1C, 0x822E89B0).
const SEEN_HALF: f32 = 0.75;
/// The least threat a vehicle is rated, whatever it carries (0x82B7D82C,
/// 0x822D56B0).
pub const VEHICLE_THREAT: f32 = 0.6;

/// What a mind or a subject is, as its `ObjectHint` has it (the threat
/// rating's table, store 0x836550BC): @0 its kind (2 a vehicle, 3 a
/// person), @4 its domain (2 ground, 3 air), @8 its armour class (1 none
/// to 4 heavy). Kind and domain named from the values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hint {
    pub kind: u32,
    pub domain: u32,
    pub armour: u32,
}

/// A soldier on foot (the `human` template's `ObjectHint`: 3, 2, 1).
pub const HUMAN: Hint = Hint {
    kind: 3,
    domain: 2,
    armour: 1,
};

/// How much of a threat weapon `w` is to `to` (0x822EC060): its rating
/// for `to`'s armour class, plus one, a quarter each (a rating of 0, a
/// tenth); times 0 for a weapon for another kind (both 2 or more and
/// different), 0.3 for another domain, 0.9 where either side is 1 (any).
pub fn weapon_threat(w: &crate::weapons::FireHint, to: Hint) -> f32 {
    let fits = |a: u32, b: u32, other: f32| match (a >= 2 && b >= 2, a == b) {
        (true, true) => 1.0,
        (true, false) => other,
        (false, _) => 0.9,
    };
    let kind = fits(w.kinds[0], to.kind, 0.0);
    let domain = fits(w.kinds[1], to.domain, 0.3);
    let rating = match to.armour {
        1..=4 => w.against[to.armour as usize - 1],
        _ => 0,
    };
    let base = if rating != 0 {
        (rating + 1) as f32 * 0.25
    } else {
        BASE
    };
    base * domain * kind
}

/// A subject's threat to `to` from the weapons it carries `distance`
/// metres off (0x822EC430 by 0x822EC1B8): the most any rates, of those
/// whose `FireRange` reaches; a vehicle's (`vehicle`) reach anywhere, and
/// it rates at least [`VEHICLE_THREAT`].
pub fn threat<'a>(
    weapons: impl IntoIterator<Item = &'a crate::weapons::FireHint>,
    to: Hint,
    distance: f32,
    vehicle: bool,
) -> f32 {
    let most = weapons
        .into_iter()
        .filter(|w| vehicle || distance < w.range)
        .map(|w| weapon_threat(w, to))
        .fold(0.0, f32::max);
    if vehicle {
        most.max(VEHICLE_THREAT)
    } else {
        most
    }
}

/// How well a subject `distance` metres off is seen by a mind that can see
/// it from `reach` metres (0x822E89B0's +0x68): a half at
/// [`SEEN_HALF`] of the reach, toward 1 near and 0 far.
pub fn visible(distance: f32, reach: f32) -> f32 {
    let r = SEEN_HALF * reach;
    if r <= 0.0 {
        return 0.0;
    }
    r * r / (r * r + distance * distance)
}
/// What a priority target is worth, in place of its threat (0x820F9560):
/// enough that the score is at its [`MOST`] whenever it is seen at all
/// well.
pub const PRIORITY: f32 = 9.8;
/// The object property that makes one a priority target, by its name
/// hash: `"PriorityTarget"`, set by the scripts' `Ai.SetPriorityTarget`
/// (0x82311B98) and read by 0x822E0988 through the property lookup
/// 0x823DA3C8.
pub const PRIORITY_PROPERTY: u32 = 0xe60c_6ca2;

/// What a subject is worth to a mind before how well it is seen: its
/// `threat` ([`threat`]) plus [`BASE`], or [`PRIORITY`] in its place for a
/// priority target.
pub fn worth(threat: f32, priority: bool) -> f32 {
    if priority { PRIORITY } else { threat + BASE }
}

/// A mind's heat (+0x1730, 0x822D9C10): up 1 each update it has someone
/// in its attack slot, to 10; down 0.5 each update it has none, to 0.
/// While any is left its scores count half again ([`Heat::factor`], the
/// mind's +0x804). Counted per update, not per second, as the 360 does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Heat(pub f32);

/// How far a mind's heat climbs while it has a target (0x820BAEEC).
const HEAT_UP_TO: f32 = 10.0;
/// What a mind's scores count by while it is hot (0x820BB6AC).
const HOT: f32 = 1.5;

impl Heat {
    /// One update, `engaged` if the mind has someone in its attack slot.
    /// (The 360 also holds the heat in one mind state, 0x17, not named
    /// yet; that is left out.)
    pub fn tick(&mut self, engaged: bool) {
        if !engaged {
            self.0 -= 0.5;
        } else if self.0 < HEAT_UP_TO {
            self.0 += 1.0;
        }
        self.0 = self.0.clamp(0.0, 500.0);
    }

    /// What the mind's scores count by: [`HOT`] while any heat is left
    /// (above 0.0001), else 1.
    pub fn factor(&self) -> f32 {
        if self.0 > 1e-4 { HOT } else { 1.0 }
    }
}

/// Within this a subject counts double, metres (0x820BB1F8).
const NEAR: f32 = 30.0;
/// As [`NEAR`], for the one already in the slot (0x820B7FB8).
const HOLD: f32 = 50.0;
/// The most a score comes to (0x820D3254).
const MOST: f32 = 0.9;
/// What being nearer than the one in the slot adds (0x820B7AE0).
const CLOSER: f32 = 0.01;
/// A score's weight in the slot (0x820BAF84).
const WEIGHT: f32 = 5.0;

/// Whether a fighter whose faction has this relation to a subject's may
/// attack them.
pub fn attacks(relation: f32) -> bool {
    relation < ATTACK_BELOW
}

/// Score of a subject `distance` metres away, `worth` this much
/// ([`worth`]) and seen this well ([`visible`], times the mind's
/// [`Heat::factor`]); `current` is the distance to the one in the slot, if
/// there is one, and `held` that this is it.
pub fn score(distance: f32, current: Option<f32>, held: bool, worth: f32, seen: f32) -> f32 {
    let near = if held { HOLD } else { NEAR };
    let base = worth * seen;
    let doubled = if distance < near { base * 2.0 } else { base };
    let closer = current.is_some_and(|c| distance < c);
    doubled.min(MOST) + if closer { CLOSER } else { 0.0 }
}

/// The mind's attack slot: who is in it and at what weight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slot {
    /// Who, by the caller's number.
    pub who: Option<u32>,
    weight: f32,
}

impl Slot {
    /// Offers `who` at `score` ([`score`]): the one in the slot takes the
    /// new weight, another takes the slot only with a greater one.
    pub fn offer(&mut self, who: u32, score: f32) {
        let weight = score * WEIGHT;
        if self.who == Some(who) || weight > self.weight {
            self.who = Some(who);
            self.weight = weight;
        }
    }

    /// Empties the slot.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weapons::FireHint;

    fn hint(kinds: [u32; 2], against: [u32; 4], range: f32) -> FireHint {
        FireHint {
            min_range: 1.0,
            range,
            angle: 0,
            on: 0.3,
            off: 4.5,
            on_close: -1.0,
            off_close: 0.75,
            against,
            single: false,
            kinds,
        }
    }

    // The game's own: the carbine (3, 1; 3, 1, 0, 0), the AT rocket
    // (1, 2; 1, 3, 3, 3), and a mounted gun (1, 1; 3, ...).
    #[test]
    fn a_weapon_threatens_what_it_is_for() {
        let carbine = hint([3, 1], [3, 1, 0, 0], 70.0);
        let rocket = hint([1, 2], [1, 3, 3, 3], 120.0);
        let mounted = hint([1, 1], [3, 1, 1, 1], 75.0);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        // Rifle at a man: (3 + 1) / 4, any domain 0.9.
        assert!(close(weapon_threat(&carbine, HUMAN), 0.9));
        // Rifle at a car: not for vehicles at all.
        let car = Hint {
            kind: 2,
            domain: 2,
            armour: 1,
        };
        assert_eq!(weapon_threat(&carbine, car), 0.0);
        // Rocket at a man: (1 + 1) / 4 x 0.9; at a tank, 0.9.
        assert!(close(weapon_threat(&rocket, HUMAN), 0.45));
        let tank = Hint { armour: 4, ..car };
        assert!(close(weapon_threat(&rocket, tank), 0.9));
        // Mounted gun at a man: 0.81.
        assert!(close(weapon_threat(&mounted, HUMAN), 0.81));
        // Only the weapons whose range reaches; a vehicle anywhere, 0.6 at
        // the least.
        assert!(close(
            threat([&carbine, &rocket], HUMAN, 100.0, false),
            0.45
        ));
        assert!(close(threat([&carbine, &rocket], HUMAN, 50.0, false), 0.9));
        assert_eq!(threat([&carbine], HUMAN, 200.0, false), 0.0);
        assert!(close(threat([&mounted], HUMAN, 500.0, true), 0.81));
        assert!(close(threat([], HUMAN, 500.0, true), VEHICLE_THREAT));
    }

    #[test]
    fn the_seen_rating_halves_at_three_quarters_of_the_reach() {
        assert!((visible(75.0, 100.0) - 0.5).abs() < 1e-6);
        assert!(visible(10.0, 100.0) > 0.98);
        assert!(visible(300.0, 100.0) < 0.06);
        assert_eq!(visible(10.0, 0.0), 0.0);
        // An armed man near outscores an unarmed one; neither tops 0.9.
        let armed = score(20.0, None, false, worth(0.9, false), visible(20.0, 90.0));
        let unarmed = score(20.0, None, false, worth(0.0, false), visible(20.0, 90.0));
        assert!(armed > unarmed && armed <= 0.9, "{armed} {unarmed}");
    }

    #[test]
    fn hostility_is_at_once_and_calm_comes_slowly() {
        let mut a = Attitude::new(-60.0);
        assert!(a.attacks());
        // Made peace: 0.3 a second, so from -60 past -33 takes 90 s.
        let mut t = 0.0_f32;
        while a.attacks() {
            a.think(0.0, 0.1);
            t += 0.1;
        }
        assert!((t - 90.0).abs() < 0.2, "{t}");
        // It settles within a point of the relation.
        for _ in 0..2000 {
            a.think(0.0, 0.1);
        }
        assert!((-1.0..=0.0).contains(&a.0), "{}", a.0);
        // War again: at once.
        a.think(-50.0, 0.1);
        assert_eq!(a, Attitude(-50.0));
    }

    #[test]
    fn only_the_hostile_band_may_be_attacked() {
        assert!(attacks(-100.0));
        assert!(attacks(-33.1));
        assert!(!attacks(-33.0));
        assert!(!attacks(0.0));
    }

    #[test]
    fn near_counts_double_and_the_held_one_longer() {
        assert_eq!(score(20.0, None, false, worth(0.0, false), 1.0), 0.2);
        assert_eq!(score(40.0, None, false, worth(0.0, false), 1.0), 0.1);
        assert_eq!(score(40.0, Some(40.0), true, worth(0.0, false), 1.0), 0.2);
        assert!(
            score(35.0, Some(40.0), false, worth(0.0, false), 1.0)
                > score(40.0, None, false, worth(0.0, false), 1.0)
        );
    }

    #[test]
    fn a_challenger_needs_more_weight_than_the_one_held() {
        let mut slot = Slot::default();
        slot.offer(1, score(40.0, None, false, worth(0.0, false), 1.0));
        slot.offer(2, score(45.0, Some(40.0), false, worth(0.0, false), 1.0));
        assert_eq!(slot.who, Some(1));
        slot.offer(3, score(35.0, Some(40.0), false, worth(0.0, false), 1.0));
        assert_eq!(slot.who, Some(3));
        slot.offer(1, score(10.0, Some(35.0), false, worth(0.0, false), 1.0));
        assert_eq!(slot.who, Some(1));
        slot.clear();
        assert_eq!(slot.who, None);
    }

    // A priority target is worth 9.8: seen at all well, it scores the most
    // there is, over the nearest rifleman.
    #[test]
    fn a_priority_target_outscores_anyone() {
        let seen = visible(150.0, 200.0);
        let heli = score(150.0, None, false, worth(VEHICLE_THREAT, true), seen);
        let rifleman = score(10.0, None, false, worth(0.9, false), 1.0);
        assert_eq!(heli, MOST);
        assert!(heli >= rifleman, "{heli} vs {rifleman}");
        assert!((worth(0.9, false) - 1.0).abs() < 1e-6);
        assert_eq!(
            PRIORITY_PROPERTY,
            m2_assets::hash::name_hash("PriorityTarget")
        );
    }

    // Heat: up 1 an update with a target, to 10; down 0.5 without. Hot,
    // scores count 1.5 times; cold, once.
    #[test]
    fn a_mind_with_a_target_runs_hot_and_cools_after() {
        let mut heat = Heat::default();
        assert_eq!(heat.factor(), 1.0);
        for _ in 0..30 {
            heat.tick(true);
        }
        assert_eq!(heat.0, 10.0);
        assert_eq!(heat.factor(), 1.5);
        for _ in 0..19 {
            heat.tick(false);
        }
        assert_eq!(heat.factor(), 1.5, "0.5 left");
        heat.tick(false);
        assert_eq!(heat.0, 0.0);
        assert_eq!(heat.factor(), 1.0);
    }
}
