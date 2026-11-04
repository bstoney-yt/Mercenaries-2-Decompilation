//! How a person's AI chooses what to do: the 360's goal planner. From the
//! 360 code unless a line says otherwise.
//!
//! A person's mind (stepped by 0x822D90D8) takes the next request off its
//! queue (attack, move to, take cover, ...) and turns it into goal state
//! bits ([`Request::goal`], 0x822DC468). The planner (0x822EA808,
//! recursing through 0x822EA9A8) then chains tasks backwards from the goal:
//! for each goal bit not yet true it tries the tasks whose effects set it
//! (lists per bit and value, built at 0x822B5570), whose own preconditions
//! become goals in turn, at most [`MAX_DEPTH`] deep, keeping the cheapest
//! chain. Each task's cost is twice its weight (0x822E9C80 sets the
//! multiplier to 1.0, and the planner adds 1 to it). The tasks, their
//! preconditions, effects and weights are the program's task table (the
//! descriptors registered at 0x822B5638), the on-foot ones of which are
//! [`TASKS`].
//!
//! State bits are numbered as the program numbers them, from 1. What a bit
//! means is inferred from the tasks that set it (named in [`bit`]).

/// State bits, named by what sets them (inferred from the tasks' effects).
pub mod bit {
    /// Standing (`HumanStand`).
    pub const STANDING: u8 = 1;
    /// On foot, not in a vehicle (`HumanExit` sets it, `HumanBoard`
    /// clears it).
    pub const ON_FOOT: u8 = 5;
    /// In a vehicle (`HumanBoard`).
    pub const IN_VEHICLE: u8 = 10;
    /// Armed (`HumanTakeWeapon`).
    pub const ARMED: u8 = 13;
    /// Aimed (`HumanAim`).
    pub const AIMED: u8 = 15;
    /// In cover (`HumanTakeCover`).
    pub const IN_COVER: u8 = 16;
    /// Stopped, walking, running (`HumanStop`, `HumanWalk`, `HumanRun`).
    pub const STOPPED: u8 = 21;
    pub const WALKING: u8 = 22;
    pub const RUNNING: u8 = 23;
    /// At the destination (`HumanWalk`, `HumanRun`).
    pub const ARRIVED: u8 = 25;
    /// Fled (`HumanFlee`).
    pub const FLED: u8 = 26;
    /// Investigated (`HumanInvestigate`).
    pub const INVESTIGATED: u8 = 28;
    /// Faced the target (`HumanTurn`).
    pub const FACED: u8 = 32;
    /// Seated (`HumanSeat`).
    pub const SEATED: u8 = 34;
    /// Fought (`HumanCombat`).
    pub const FOUGHT: u8 = 35;
    /// Took the thing (`HumanTake`, `HumanTakeWeapon`).
    pub const TOOK: u8 = 36;
    /// Used the thing (`HumanUse`).
    pub const USED: u8 = 37;
    /// Observed (`HumanObserve`).
    pub const OBSERVED: u8 = 43;
    /// Walked the sidewalks, idled (`HumanWalkSidewalks`, `HumanIdle`).
    pub const STROLLED: u8 = 53;
    pub const IDLED: u8 = 54;
    /// Conversed (`HumanConverse`).
    pub const CONVERSED: u8 = 59;
}

/// A world state: which bits are known and their values (the planner's
/// pair of 64-bit masks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub known: u64,
    pub value: u64,
}

impl State {
    fn mask(b: u8) -> u64 {
        1u64 << (b - 1)
    }

    /// The state with bit `b` set to `on`.
    pub fn with(mut self, b: u8, on: bool) -> Self {
        let m = Self::mask(b);
        self.known |= m;
        if on {
            self.value |= m;
        } else {
            self.value &= !m;
        }
        self
    }

    /// Bit `b`, if known.
    pub fn get(&self, b: u8) -> Option<bool> {
        let m = Self::mask(b);
        (self.known & m != 0).then_some(self.value & m != 0)
    }

    /// Whether every bit `goal` knows has its value here.
    pub fn satisfies(&self, goal: &State) -> bool {
        goal.known & !self.known == 0 && (self.value ^ goal.value) & goal.known == 0
    }
}

/// A task the planner chains: its name (the program's), what must hold
/// for it, what it makes hold, and its weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    pub name: &'static str,
    pub pre: &'static [(u8, bool)],
    pub fx: &'static [(u8, bool)],
    pub weight: f32,
}

impl Task {
    /// What choosing it costs (twice its weight).
    pub fn cost(&self) -> f32 {
        2.0 * self.weight
    }
}

macro_rules! task {
    ($name:literal, $w:literal, [$($p:literal = $pv:literal),*], [$($f:literal = $fv:literal),*]) => {
        Task {
            name: $name,
            weight: $w,
            pre: &[$(($p, $pv != 0)),*],
            fx: &[$(($f, $fv != 0)),*],
        }
    };
}

/// The on-foot tasks of the 360's table (descriptors 0x82B78760 to
/// 0x82B7AA40: preconditions at +0x5c, effects at +0xdc, weight at +0x11c).
pub const TASKS: [Task; 30] = [
    task!("HumanTake", 1.5, [5 = 1, 25 = 1, 21 = 1], [36 = 1]),
    task!(
        "HumanTakeWeapon",
        1.5,
        [5 = 1, 45 = 1, 25 = 1, 21 = 1],
        [36 = 1, 13 = 1, 14 = 1]
    ),
    task!(
        "HumanWalk",
        12.0,
        [5 = 1, 1 = 1],
        [25 = 1, 32 = 0, 21 = 0, 22 = 1, 23 = 0, 24 = 0]
    ),
    task!(
        "HumanRun",
        8.0,
        [5 = 1, 1 = 1],
        [25 = 1, 32 = 0, 21 = 0, 22 = 0, 23 = 1, 24 = 0]
    ),
    task!("HumanWalkSidewalks", 0.5, [5 = 1], [53 = 1]),
    task!("HumanIdle", 0.5, [5 = 1], [54 = 1]),
    task!("HumanDance", 0.5, [5 = 1], [55 = 1]),
    task!("HumanExercise", 0.5, [5 = 1], [56 = 1]),
    task!("HumanConverse", 0.5, [5 = 1, 32 = 1], [59 = 1]),
    task!("HumanStop", 0.5, [5 = 1], [21 = 1, 22 = 0, 23 = 0, 24 = 0]),
    task!("HumanUse", 3.0, [5 = 1], [37 = 1]),
    task!(
        "HumanBoard",
        1.0,
        [47 = 1, 5 = 1, 1 = 1, 25 = 1, 21 = 1, 22 = 0, 23 = 0, 24 = 0],
        [5 = 0, 6 = 1, 10 = 1]
    ),
    task!("HumanScan", 0.5, [5 = 1, 1 = 1, 21 = 1], [30 = 1]),
    task!("HumanAction", 0.5, [5 = 1, 1 = 1, 21 = 1], [31 = 1]),
    task!("HumanTurn", 0.5, [5 = 1, 1 = 1, 21 = 1], [32 = 1]),
    task!("HumanAlign", 0.5, [5 = 1, 1 = 1, 21 = 1], [33 = 1]),
    task!("HumanStand", 0.5, [5 = 1], [1 = 1, 2 = 0, 3 = 0]),
    task!(
        "HumanExit",
        5.0,
        [5 = 0, 4 = 0, 57 = 1],
        [5 = 1, 6 = 0, 7 = 0, 8 = 0, 9 = 0, 10 = 0]
    ),
    task!("HumanAim", 0.75, [5 = 1, 13 = 1, 21 = 1], [15 = 1]),
    task!("HumanCombat", 0.5, [5 = 1, 13 = 1], [35 = 1]),
    task!("HumanAvoid", 5.0, [5 = 1], [17 = 1]),
    task!("HumanTakeCover", 3.0, [5 = 1], [16 = 1]),
    task!("HumanSateRest", 4.0, [5 = 1, 49 = 1, 37 = 1], [51 = 1]),
    task!("HumanFlee", 4.0, [5 = 1], [26 = 1]),
    task!("HumanObserve", 1.0, [25 = 1, 32 = 1, 21 = 1], [43 = 1]),
    task!("HumanSeat", 0.5, [5 = 1, 25 = 1, 21 = 1, 32 = 1], [34 = 1]),
    task!("HumanRadioIn", 1.0, [5 = 1, 1 = 1], []),
    task!("MindPathMove", 1.0, [25 = 1], []),
    task!("MindIdle", 4.0, [], [54 = 1]),
    task!(
        "HumanInvestigate",
        1.5,
        [5 = 1, 21 = 1, 33 = 1],
        [28 = 1, 32 = 0, 21 = 0, 22 = 0, 23 = 1, 24 = 0]
    ),
];

/// The state bits a task must not set against what is wanted (0x5f0).
const WHERE: u64 = 0x5f0;

/// The planner's deepest chain (0x822EA9A8 gives up past 12 levels).
pub const MAX_DEPTH: usize = 12;

/// A request on a person's mind's queue, by the type number the program
/// gives it, and the goal it becomes (0x822DC468; only the requests whose
/// goal is a plain set of bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// 1: take a thing.
    Take,
    /// 2: go to a thing and stop there.
    GoTo,
    /// 4: face a thing.
    Face,
    /// 5: flee from a thing.
    Flee,
    /// 9: use a thing.
    Use,
    /// 10: move to a place.
    MoveTo,
    /// 13 (0xd): walk the sidewalks.
    Stroll,
    /// 14 (0xe): observe a thing.
    Observe,
    /// 15 (0xf): sit in a seat.
    Sit,
    /// 16 (0x10): stand still.
    StandStill,
    /// 17 (0x11): idle where they are.
    Idle,
    /// 18 (0x12): board a vehicle.
    Board,
    /// 19 (0x13): converse.
    Converse,
    /// 20 (0x14): aim at a thing.
    Aim,
    /// 21 (0x15): attack a thing.
    Attack,
    /// 22 (0x16): take cover from a thing.
    TakeCover,
    /// 23 (0x17): investigate a place.
    Investigate,
}

impl Request {
    /// The goal state the request asks for.
    pub fn goal(self) -> State {
        let bits: &[u8] = match self {
            Request::Take => &[bit::TOOK],
            Request::GoTo => &[bit::STOPPED, bit::ARRIVED],
            Request::Face => &[bit::FACED],
            Request::Flee => &[bit::FLED, 57],
            Request::Use => &[bit::USED],
            Request::MoveTo => &[bit::ARRIVED],
            Request::Stroll => &[bit::STROLLED],
            Request::Observe => &[bit::OBSERVED],
            Request::Sit => &[bit::SEATED],
            Request::StandStill => &[bit::STANDING, bit::STOPPED],
            Request::Idle => &[bit::IDLED],
            Request::Board => &[bit::IN_VEHICLE],
            Request::Converse => &[bit::CONVERSED],
            Request::Aim => &[bit::AIMED],
            Request::Attack => &[bit::FOUGHT],
            Request::TakeCover => &[bit::IN_COVER],
            Request::Investigate => &[bit::INVESTIGATED],
        };
        bits.iter().fold(State::default(), |s, &b| s.with(b, true))
    }
}

/// The cheapest chain of tasks from `now` to `goal` (first to do first),
/// with its cost; `None` when no chain within [`MAX_DEPTH`] gets there.
pub fn plan(now: State, goal: State) -> Option<(Vec<&'static Task>, f32)> {
    let mut best: Option<(Vec<&'static Task>, f32)> = None;
    let mut chain = Vec::new();
    search(now, goal, 0.0, &mut chain, &mut best);
    best.map(|(mut c, cost)| {
        c.reverse();
        (c, cost)
    })
}

/// Backwards from `goal`: each step picks a task that sets an unmet goal
/// bit to its wanted value without undoing another goal bit, and its
/// preconditions join what is still wanted.
fn search(
    now: State,
    goal: State,
    cost: f32,
    chain: &mut Vec<&'static Task>,
    best: &mut Option<(Vec<&'static Task>, f32)>,
) {
    if best.as_ref().is_some_and(|(_, b)| cost >= *b) {
        return;
    }
    // What is still wanted: the goal bits `now` doesn't already give.
    let unmet = goal.known & !(now.known & !(now.value ^ goal.value));
    if unmet == 0 {
        *best = Some((chain.clone(), cost));
        return;
    }
    if chain.len() >= MAX_DEPTH {
        return;
    }
    let want = unmet.trailing_zeros() as u8 + 1;
    let wanted = goal.get(want).unwrap_or(true);
    for t in TASKS.iter() {
        if !t.fx.iter().any(|&(b, v)| b == want && v == wanted) {
            continue;
        }
        // Only where the person is (on foot, in which seat) must agree
        // with what is still wanted: the planner checks the state bits
        // 0x5f0 masks alone (5 to 9 and 11) and lets a chain undo the
        // rest, which the mind plans again for once the chain is done.
        if t.fx
            .iter()
            .any(|&(b, v)| WHERE & State::mask(b) != 0 && goal.get(b).is_some_and(|g| g != v))
        {
            continue;
        }
        if chain.iter().any(|c| std::ptr::eq(*c, t)) {
            continue;
        }
        // What remains: the goal less what it sets, plus what it needs.
        let mut next = goal;
        for &(b, _) in t.fx {
            let m = State::mask(b);
            next.known &= !m;
            next.value &= !m;
        }
        let mut clash = false;
        for &(b, v) in t.pre {
            if next.get(b).is_some_and(|g| g != v) {
                clash = true;
            }
            next = next.with(b, v);
        }
        if clash {
            continue;
        }
        chain.push(t);
        search(now, next, cost + t.cost(), chain, best);
        chain.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing_armed() -> State {
        State::default()
            .with(bit::ON_FOOT, true)
            .with(bit::STANDING, true)
            .with(bit::ARMED, true)
            .with(bit::STOPPED, true)
    }

    fn names(p: &(Vec<&'static Task>, f32)) -> Vec<&'static str> {
        p.0.iter().map(|t| t.name).collect()
    }

    #[test]
    fn an_armed_soldier_asked_to_attack_fights() {
        let p = plan(standing_armed(), Request::Attack.goal()).unwrap();
        assert_eq!(names(&p), ["HumanCombat"]);
        assert_eq!(p.1, 1.0);
    }

    #[test]
    fn moving_to_a_place_he_runs_the_cheaper_way() {
        // Running (weight 8) is cheaper than walking (12).
        let p = plan(standing_armed(), Request::MoveTo.goal()).unwrap();
        assert_eq!(names(&p), ["HumanRun"]);
        // Stopped already, going there and stopping is a run (the stop
        // comes from the next plan, once there); on the move, a stop too.
        let p = plan(standing_armed(), Request::GoTo.goal()).unwrap();
        assert_eq!(names(&p), ["HumanRun"]);
        let moving = standing_armed().with(bit::STOPPED, false);
        let p = plan(moving, Request::GoTo.goal()).unwrap();
        assert_eq!(names(&p), ["HumanRun", "HumanStop"]);
    }

    #[test]
    fn aiming_on_the_move_he_stops_first() {
        let moving = standing_armed().with(bit::STOPPED, false);
        let p = plan(moving, Request::Aim.goal()).unwrap();
        assert_eq!(names(&p), ["HumanStop", "HumanAim"]);
    }

    #[test]
    fn unarmed_he_cannot_fight_without_a_weapon_to_take() {
        let unarmed = standing_armed().with(bit::ARMED, false);
        assert!(plan(unarmed, Request::Attack.goal()).is_none());
        // With one lying there (bit 45), he runs to it, takes it, fights.
        let p = plan(unarmed.with(45, true), Request::Attack.goal()).unwrap();
        assert_eq!(names(&p), ["HumanRun", "HumanTakeWeapon", "HumanCombat"]);
    }

    #[test]
    fn cover_and_flight_are_one_task_each() {
        let p = plan(standing_armed(), Request::TakeCover.goal()).unwrap();
        assert_eq!(names(&p), ["HumanTakeCover"]);
        assert_eq!(p.1, 6.0);
    }
}
