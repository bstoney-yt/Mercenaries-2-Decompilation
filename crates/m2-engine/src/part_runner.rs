//! A model's part state machines run over time once its object is
//! destroyed (a burning canopy coming down a piece at a time): the
//! messages each state takes, the states it sets, the messages it sends to
//! the model's other parts and the states it sets later, each part's
//! changes told by its node's name (the machine the scripts'
//! `OnStateChange` is told of: `Slice00`, a canopy's crown).
//!
//! **From the 360 code and the models' state machines** (`STAM`, read by
//! [`m2_assets::model::PartStates`]):
//!
//! - A part destroyed takes `DestroyMsg` or `0x3D0D4C99` (0x82289020,
//!   [`crate::destroy_route`]); a state's `0xE1142510` (`ON_MESSAGE`)
//!   names the state a message sends it to (0: none).
//! - Entering a state runs its commands in order: `SetState` on another
//!   part, `SendMessage` to another part ([`StateActs::told`]), and
//!   `0xBD3403A0`, a `SetState` after a delay of `a + r (b - a)` seconds
//!   (0x82231C58, `r` the engine's random fraction; [`StateActs::delayed`]),
//!   posted to the timed queue 0x83568398.
//! - The large jungle canopy (`jungle_env_largecanopy01`): its trunk,
//!   destroyed either way, enters `CollapseFireState` (a fire on node 4),
//!   tells the crown `Slice00` `0x1B071983` and is gone 2.5 to 3 s later;
//!   the crown goes to `0x3CC4E770` on that message and comes down
//!   (`CollapseFireState`, its burnt material's cue) 3 to 12 s later, and
//!   is destroyed 3 to 4.1 s after that.
//!
//! **Inferred:** a machine starts in its `InitState`'s state (the part
//! whole), told of no change; the random fraction is this module's own
//! (a fixed sequence per object, [`Runner::new`]'s `seed`), as the engine's
//! generator (0x8215E260) isn't followed.

use m2_assets::model::{PartStates, StateActs};

/// A part's state change, told by its machine's (node's) name hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub machine: u32,
    pub state: u32,
}

#[derive(Clone, Debug)]
struct Machine {
    node: Option<u16>,
    name: u32,
    states: Vec<u32>,
    acts: Vec<StateActs>,
    current: Option<u32>,
}

/// The part machines of one object, from its destruction on.
#[derive(Clone, Debug)]
pub struct Runner {
    machines: Vec<Machine>,
    /// States due later: (when, state, part's node).
    timers: Vec<(f32, u32, u16)>,
    now: f32,
    seed: u32,
    changes: Vec<Change>,
}

/// How deep a chain of states set by states may go (a guard against a
/// machine that sets itself round for ever).
const DEPTH: usize = 16;

impl Runner {
    /// `states`' machines, each in the state its `InitState` gives it;
    /// `node_name` is each node's name hash, `seed` the object's own.
    pub fn new(states: &PartStates, node_name: impl Fn(u16) -> u32, seed: u32) -> Self {
        let machines = states
            .machines
            .iter()
            .map(|m| Machine {
                node: m.node,
                name: m.node.map_or(0, &node_name),
                states: m.states.iter().map(|s| s.0).collect(),
                acts: m.acts.clone(),
                current: m.starts[0],
            })
            .collect();
        Self {
            machines,
            timers: Vec::new(),
            now: 0.0,
            seed: seed | 1,
            changes: Vec::new(),
        }
    }

    /// A random fraction, 0 to 1 (xorshift).
    fn fraction(&mut self) -> f32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Every part takes `message` (`DestroyMsg` or the fire-destroy
    /// message, as the object is destroyed).
    pub fn deliver(&mut self, message: u32) {
        for m in 0..self.machines.len() {
            self.deliver_to(m, message, 0);
        }
    }

    fn deliver_to(&mut self, m: usize, message: u32, depth: usize) {
        let Some(current) = self.machines[m].current else {
            return;
        };
        let Some(k) = self.machines[m].states.iter().position(|&s| s == current) else {
            return;
        };
        let to = self.machines[m].acts.get(k).and_then(|a| {
            a.on.iter()
                .find(|(msg, _)| *msg == message)
                .map(|&(_, to)| to)
        });
        if let Some(to) = to.filter(|&to| to != 0) {
            self.enter(m, to, depth);
        }
    }

    fn by_node(&self, node: u16) -> Option<usize> {
        self.machines.iter().position(|m| m.node == Some(node))
    }

    fn enter(&mut self, m: usize, state: u32, depth: usize) {
        if depth > DEPTH {
            return;
        }
        let machine = &mut self.machines[m];
        let Some(k) = machine.states.iter().position(|&s| s == state) else {
            return;
        };
        machine.current = Some(state);
        self.changes.push(Change {
            machine: machine.name,
            state,
        });
        let acts = machine.acts.get(k).cloned().unwrap_or_default();
        for &(to, node) in &acts.sets {
            if let Some(other) = self.by_node(node) {
                self.enter(other, to, depth + 1);
            }
        }
        for &(message, node) in &acts.told {
            if let Some(other) = self.by_node(node) {
                self.deliver_to(other, message, depth + 1);
            }
        }
        for &(to, node, a, b) in &acts.delayed {
            let r = self.fraction();
            self.timers.push((self.now + a + r * (b - a), to, node));
        }
    }

    /// Time passes: the states due come in, in time order.
    pub fn advance(&mut self, dt: f32) {
        self.now += dt;
        self.timers.sort_by(|a, b| a.0.total_cmp(&b.0));
        while let Some(&(when, to, node)) = self.timers.first() {
            if when > self.now {
                break;
            }
            self.timers.remove(0);
            if let Some(m) = self.by_node(node) {
                self.enter(m, to, 0);
            }
        }
    }

    /// The changes since the last call, in order.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }

    /// Whether nothing more is due.
    pub fn settled(&self) -> bool {
        self.timers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::model::{DESTROY_MSG, PartMachine};

    const PRISTINE: u32 = 0xacb5_1200;
    const COLLAPSE_FIRE: u32 = 0x0753_c8f7;
    const WAITING: u32 = 0x3cc4_e770;
    const DESTROYED: u32 = 0x7687_df41;
    const TOLD: u32 = 0x1b07_1983;

    fn machine(node: u16, states: Vec<(u32, StateActs)>) -> PartMachine {
        PartMachine {
            node: Some(node),
            states: states.iter().map(|(s, _)| (*s, Vec::new())).collect(),
            starts: [Some(PRISTINE), None],
            acts: states.into_iter().map(|(_, a)| a).collect(),
        }
    }

    /// The large jungle canopy's two machines, as its `STAM` has them.
    fn canopy() -> PartStates {
        let crown = machine(
            3,
            vec![
                (
                    PRISTINE,
                    StateActs {
                        on: vec![(TOLD, WAITING)],
                        ..Default::default()
                    },
                ),
                (
                    WAITING,
                    StateActs {
                        delayed: vec![(COLLAPSE_FIRE, 3, 3.0, 12.0)],
                        ..Default::default()
                    },
                ),
                (
                    COLLAPSE_FIRE,
                    StateActs {
                        delayed: vec![(DESTROYED, 3, 3.0, 4.1)],
                        ..Default::default()
                    },
                ),
                (DESTROYED, StateActs::default()),
            ],
        );
        let trunk = machine(
            1,
            vec![
                (
                    PRISTINE,
                    StateActs {
                        on: vec![(DESTROY_MSG, COLLAPSE_FIRE), (0x3d0d_4c99, COLLAPSE_FIRE)],
                        ..Default::default()
                    },
                ),
                (
                    COLLAPSE_FIRE,
                    StateActs {
                        told: vec![(TOLD, 3)],
                        delayed: vec![(WAITING, 1, 2.5, 3.0)],
                        ..Default::default()
                    },
                ),
                (
                    WAITING,
                    StateActs {
                        sets: vec![(DESTROYED, 1)],
                        ..Default::default()
                    },
                ),
                (DESTROYED, StateActs::default()),
            ],
        );
        PartStates {
            machines: vec![crown, trunk],
            ..Default::default()
        }
    }

    const SLICE00: u32 = 0x4a59_4fe9;
    const TRUNK: u32 = 0x501d_5fd7;

    fn name(n: u16) -> u32 {
        if n == 3 { SLICE00 } else { TRUNK }
    }

    #[test]
    fn a_destroyed_canopy_comes_down_a_piece_at_a_time() {
        for seed in [1, 7, 99, 12345] {
            let mut r = Runner::new(&canopy(), name, seed);
            r.deliver(DESTROY_MSG);
            let change = |machine, state| Change { machine, state };
            assert_eq!(
                r.take_changes(),
                [change(TRUNK, COLLAPSE_FIRE), change(SLICE00, WAITING)]
            );
            // When each came, ticking 0.1 s.
            let mut seen: Vec<(f32, Change)> = Vec::new();
            for i in 1..=200 {
                r.advance(0.1);
                let t = 0.1 * i as f32;
                seen.extend(r.take_changes().into_iter().map(|c| (t, c)));
            }
            let when = |c: Change| seen.iter().find(|s| s.1 == c).map(|s| s.0).unwrap();
            // The trunk is gone 2.5 to 3 s on.
            let gone = when(change(TRUNK, DESTROYED));
            assert!((2.5..=3.1).contains(&gone), "{gone}");
            assert_eq!(when(change(TRUNK, WAITING)), gone);
            // The crown comes down 3 to 12 s after it was told, then is
            // gone 3 to 4.1 s after that.
            let down = when(change(SLICE00, COLLAPSE_FIRE));
            let out = when(change(SLICE00, DESTROYED));
            assert!((3.0..=12.1).contains(&down), "{down}");
            assert!((2.9..=4.2).contains(&(out - down)), "{out} {down}");
            assert!(r.settled());
        }
    }

    #[test]
    fn a_message_a_state_drops_or_does_not_know_changes_nothing() {
        let mut r = Runner::new(&canopy(), name, 1);
        r.deliver(0x1234_5678);
        assert!(r.take_changes().is_empty());
    }
}
