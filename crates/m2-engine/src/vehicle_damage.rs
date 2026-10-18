//! A vehicle's parts taking damage one at a time, as its template and its
//! model's state machine say: a car's doors dent at half their health and
//! come off at none, its glass breaks, its hood catches fire as the car
//! burns down, and its wreck smokes.
//!
//! - Each part's health comes from the template's `NodeHealth` records:
//!   the part's node (by name hash), its health, a scale (`27c50f84`,
//!   name unknown, 1 on every car sampled) and the health at which it is
//!   damaged (`ea813967`, name unknown: 15 of a door's 30, 16 of the
//!   hood's 20, 0 for parts that only break).
//! - What a part looks like and does in each state comes from the model's
//!   state machine ([`PartStates`], [`StateActs`]): `PristineState` shows
//!   the whole door and sends the part on `DamageMsg` to a state showing
//!   the dented door, and on `DestroyMsg` to one that throws the door off
//!   (`CreateObject` naming `DetachState`) and then shows nothing.
//! - The body (the part `TickDamage` names) is damaged when the vehicle's
//!   health falls to its `TickDamage` threshold, where it starts to burn
//!   down: its `DamagedState` starts `global_particle_fire_carhood` at
//!   the hood. It is destroyed when the vehicle is wrecked: the wreck's
//!   state starts its smoke and fire and sends the glass and the other
//!   parts to the states that show their wrecks.
//!
//! # Provenance
//!
//! From samples: the records and the state machines of the R90, the
//! Valiant and the M1A2 of a legally installed copy (round 14). Read off
//! the data, not checked against the 360 code:
//!
//! - That `DamageMsg` comes at the `ea813967` health and `DestroyMsg` at
//!   none, and that the body is damaged at the `TickDamage` health.
//! - That a message's handler stays set in later states until a state
//!   sets it again (the dented door's state sets only `DamageMsg`, to
//!   nothing; its `DestroyMsg` still throws the door off).
//! - How much of a hit a part takes: the hit's own damage (a rifle round's
//!   9) times the part's scale, nearest part to where it struck (within
//!   [`HIT_SLACK`], invented). A blast hurts each part in reach by its
//!   own distance from the heart, by the explosion template's fall from
//!   `Damage` to `MinDamage` (round 21; before, a straight fall to
//!   nothing, invented).
//! - A part's own `ModifierKey` record (`Name` its node: 329 of the
//!   vehicle templates', `WeakPointTank` on the tanks' and IFVs',
//!   `WeakPointHeli` on the helicopters') makes it a weak point: where a
//!   hit strikes it, the damage table's row for that key stands in for
//!   the vehicle's (read off the rows: the weak points' give rockets 5 to
//!   10 times the tank's, rounds the same; not checked against the 360
//!   code).
//! - `DebrisEffect`'s fields by their hashes ([`Debris`]): its counts,
//!   speeds and times are read as such from their names; that the speed
//!   is how fast a chunk leaves and the time how long it lasts is
//!   inferred.
//!
//! Of the `NodeHealth` records, 668 (one a template, health 1000) name
//! `c851b695` ([`UNNAMED_WHOLE`]), which no model's node is; the rest
//! that name no part name nodes their template's model lacks (a template
//! sharing its records with another model's). Neither is a part.
//!
//! Not read: what the handlers on leaving a state do but hide and stop
//! emitters (a lamp passes damage to the hood as it breaks), the third
//! message the parts take (`3d0d4c99`, unnamed: it sends a part where
//! `DestroyMsg` does), `NodeHealth`'s `0ec0aee7` (0 throughout) and
//! `261b2109` (1 on 229 records).
//!
//! Not done here: bullet holes in paint and glass (the footage's 14:06).
//! No part state shows them; read as decals the game places where
//! rounds strike.

use std::collections::HashMap;

use m2_assets::model::{DAMAGE_MSG, DESTROY_MSG, Model, Node, PartStates};
use m2_assets::{Layer, name_hash};
use m2_math::glam::{Mat4, Vec3};

use crate::tuning::Table;

/// `NodeHealth`'s scale and damaged-at fields (names unknown).
const SCALE: u32 = 0x27c5_0f84;
const DAMAGED_AT: u32 = 0xea81_3967;
/// The name `NodeHealth` records give that no model's node has (668 of
/// the vehicle templates' 6,696 records, every one with health 1000; the
/// name itself unsolved). Not a part: the survey counts them apart.
pub const UNNAMED_WHOLE: u32 = 0xc851_b695;

/// One part's health, from a `NodeHealth` record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeHealth {
    /// The part's node name hash.
    pub part: u32,
    pub health: f32,
    /// Of the damage it takes, how much counts.
    pub scale: f32,
    /// The health at which it is damaged (0: never, it only breaks).
    pub damaged_at: f32,
}

/// The chunks a wreck throws, one `DebrisEffect` record (field names
/// matched to their hashes: `iMinChunks`, `iMaxChunks`, `MinChunkVel`,
/// `MaxChunkVel`, `MinChunkTime`, `MaxChunkTime`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Debris {
    /// The chunk's template (`Chunk_aa`) and that template's model, if
    /// it is debris (`_DebrisPhysics`; not the fuel can, a pickup).
    pub template: u32,
    pub model: Option<u32>,
    /// The node it flies from (`Hardpoint`; `None`: the vehicle's
    /// middle, as every record sampled has it).
    pub hardpoint: Option<u32>,
    /// How many, fewest and most.
    pub count: (u32, u32),
    /// How fast they leave, m/s, slowest and fastest (read as speeds).
    pub speed: (f32, f32),
    /// How long they last, seconds, shortest and longest (read as
    /// lifetimes).
    pub lasts: (f32, f32),
}

/// A template's parts' health, the part that burns, its parts' own
/// damage keys and the chunks its wreck throws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartHealths {
    pub parts: Vec<NodeHealth>,
    /// The part `TickDamage` names (its field `NodeName`): the body.
    pub burn_part: Option<u32>,
    /// Parts with a `ModifierKey` of their own, (node name hash, key):
    /// a tank's or helicopter's weak point (`WeakPointTank`,
    /// `WeakPointHeli`), which the damage table hurts more by rockets.
    pub keys: Vec<(u32, u32)>,
    /// What its wreck throws (`DebrisEffect`).
    pub debris: Vec<Debris>,
}

/// Each template's parts' health, by id: those with `NodeHealth`, a
/// part's own `ModifierKey` or `DebrisEffect`.
pub fn read(db: &Layer<'_>) -> HashMap<u32, PartHealths> {
    let node = Table::new(db, "NodeHealth");
    let tick = Table::new(db, "TickDamage");
    let modifier = Table::new(db, "ModifierKey");
    let effect = Table::new(db, "DebrisEffect");
    let models = db.hashes("ModelName");
    let physics = Table::new(db, "_DebrisPhysics");
    let (name, health) = (name_hash("Name"), name_hash("Health"));
    let key = name_hash("ModifierKey");
    let mut ids: Vec<u32> = node
        .templates()
        .chain(effect.templates())
        .chain(modifier.templates().filter(|&id| {
            modifier
                .of_template(id)
                .iter()
                .any(|r| modifier.word(r, name).is_some_and(|n| n != 0))
        }))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let parts = node
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    Some(NodeHealth {
                        part: node.word(r, name).filter(|&h| h != 0)?,
                        health: node.float(r, health)?,
                        scale: node.float(r, SCALE).unwrap_or(1.0),
                        damaged_at: node.float(r, DAMAGED_AT).unwrap_or(0.0),
                    })
                })
                .collect();
            let burn_part = tick
                .of_template(id)
                .first()
                .and_then(|r| tick.word(r, name_hash("NodeName")))
                .filter(|&h| h != 0);
            let keys = modifier
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let part = modifier.word(r, name).filter(|&n| n != 0)?;
                    Some((part, modifier.word(r, key).filter(|&k| k != 0)?))
                })
                .collect();
            let debris = effect
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let w = |f: &str| effect.word(r, name_hash(f));
                    let fl = |f: &str| effect.float(r, name_hash(f));
                    let template = w("Template").filter(|&t| t != 0)?;
                    let (lo, hi) = (w("iMinChunks")?, w("iMaxChunks")?);
                    Some(Debris {
                        template,
                        // Only a chunk of debris (`_DebrisPhysics`) is thrown as
                        // one; the fuel can some records name is a pickup.
                        model: models
                            .get(&template)
                            .copied()
                            .filter(|&m| m != 0 && !physics.of_template(template).is_empty()),
                        hardpoint: w("Hardpoint").filter(|&h| h != 0),
                        count: (lo.min(hi), lo.max(hi)),
                        speed: (fl("MinChunkVel")?, fl("MaxChunkVel")?),
                        lasts: (fl("MinChunkTime")?, fl("MaxChunkTime")?),
                    })
                })
                .collect();
            (
                id,
                PartHealths {
                    parts,
                    burn_part,
                    keys,
                    debris,
                },
            )
        })
        .collect()
}

/// What a model's parts can do as they are damaged: its state machine,
/// and each part's box.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageModel {
    pub states: PartStates,
    pub nodes: Vec<Node>,
    /// Each node's place in the model (node to model).
    pub rest: Vec<Mat4>,
    /// Per part (machine), the box of everything under its node, model
    /// space; `None` for a part with nothing to show.
    pub boxes: Vec<Option<(Vec3, Vec3)>>,
}

impl DamageModel {
    /// `model`'s, if its state machine has parts that take messages.
    pub fn from_model(model: &Model<'_>) -> Option<Self> {
        let states = model.states.clone()?;
        if !states
            .machines
            .iter()
            .flat_map(|m| &m.acts)
            .any(|a| !a.on.is_empty())
        {
            return None;
        }
        let rest: Vec<Mat4> = (0..model.nodes.len())
            .map(|i| model.node_to_model(i as u16).unwrap_or(Mat4::IDENTITY))
            .collect();
        let under = |top: u16| {
            (0..model.nodes.len()).filter(move |&i| {
                let mut at = Some(i as u16);
                for _ in 0..=model.nodes.len() {
                    match at {
                        Some(n) if n == top => return true,
                        Some(n) => at = model.nodes[usize::from(n)].parent,
                        None => return false,
                    }
                }
                false
            })
        };
        let boxes = states
            .machines
            .iter()
            .map(|m| {
                let top = m.node?;
                let mut b: Option<(Vec3, Vec3)> = None;
                for i in under(top) {
                    let n = &model.nodes[i];
                    let (lo, hi) = (
                        Vec3::new(n.bounds_min[0], n.bounds_min[1], n.bounds_min[2]),
                        Vec3::new(n.bounds_max[0], n.bounds_max[1], n.bounds_max[2]),
                    );
                    if (hi - lo).length_squared() < 1e-6 {
                        continue;
                    }
                    for k in 0..8 {
                        let c = Vec3::new(
                            if k & 1 == 0 { lo.x } else { hi.x },
                            if k & 2 == 0 { lo.y } else { hi.y },
                            if k & 4 == 0 { lo.z } else { hi.z },
                        );
                        let p = rest[i].transform_point3(c);
                        b = Some(b.map_or((p, p), |(a, z)| (a.min(p), z.max(p))));
                    }
                }
                b
            })
            .collect();
        Some(Self {
            states,
            nodes: model.nodes.clone(),
            rest,
            boxes,
        })
    }

    /// The part whose node is named `hash`.
    pub fn part_named(&self, hash: u32) -> Option<usize> {
        self.states.machines.iter().position(|m| {
            m.node
                .and_then(|n| self.nodes.get(usize::from(n)))
                .is_some_and(|n| n.name_hash == hash)
        })
    }

    /// The part a hit at `at` (model space) strikes: the smallest box
    /// within `slack` metres of it, but the body's (a box holding all the
    /// others) only if nothing smaller is.
    pub fn part_at(&self, at: Vec3, slack: f32) -> Option<usize> {
        self.boxes
            .iter()
            .enumerate()
            .filter_map(|(k, b)| {
                let (lo, hi) = (*b)?;
                let d = (lo - at).max(at - hi).max(Vec3::ZERO).length();
                let size = hi - lo;
                (d <= slack).then_some((k, size.x * size.y * size.z))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _)| k)
    }

    /// Model-space place of node `n`'s origin.
    pub fn node_at(&self, n: u16) -> Vec3 {
        self.rest
            .get(usize::from(n))
            .map_or(Vec3::ZERO, |m| m.w_axis.truncate())
    }
}

/// A part thrown off: its node, and its box's centre then (model space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detached {
    pub part: usize,
    pub node: u16,
    pub centre: Vec3,
}

/// How far messages chain (a state's `SetState` and `SendDamage` to
/// other parts) before the rest are dropped.
const CHAIN: u32 = 16;

/// One vehicle's parts as they stand: each part's state, health and the
/// message handlers its states have set.
#[derive(Clone, Debug, PartialEq)]
pub struct PartDamage {
    /// Per part, its state (an index into its machine's states).
    pub state: Vec<Option<usize>>,
    /// Per part, its health left and its record, if its template gives
    /// one.
    health: Vec<Option<(f32, NodeHealth)>>,
    /// Per part, where each message sends it (message, state name).
    handlers: Vec<Vec<(u32, u32)>>,
    /// Per part, whether it has taken its `DamageMsg` and `DestroyMsg`.
    sent: Vec<[bool; 2]>,
    /// Per part, its own `ModifierKey`, if its template gives one.
    keys: Vec<Option<u32>>,
    /// The body (the part `TickDamage` names).
    pub body: Option<usize>,
    /// Parts thrown off since last taken ([`take_detached`]).
    ///
    /// [`take_detached`]: Self::take_detached
    detached: Vec<Detached>,
    /// The effects running: (part, effect name hash, node), started by
    /// a state and kept up until a state the part leaves stops them.
    running: Vec<(usize, u32, u16)>,
    /// States entered by message, counted (for the logs).
    pub entered: usize,
}

impl PartDamage {
    /// `model`'s parts intact, with `healths` from its template.
    pub fn new(model: &DamageModel, healths: &PartHealths) -> Self {
        let n = model.states.machines.len();
        let mut d = Self {
            state: vec![None; n],
            health: vec![None; n],
            handlers: vec![Vec::new(); n],
            sent: vec![[false; 2]; n],
            keys: vec![None; n],
            body: healths.burn_part.and_then(|h| model.part_named(h)),
            detached: Vec::new(),
            running: Vec::new(),
            entered: 0,
        };
        for h in &healths.parts {
            if let Some(k) = model.part_named(h.part) {
                d.health[k] = Some((h.health, *h));
            }
        }
        for &(part, key) in &healths.keys {
            if let Some(k) = model.part_named(part) {
                d.keys[k] = Some(key);
            }
        }
        for (k, m) in model.states.machines.iter().enumerate() {
            let Some(s) = PartStates::intact_state(m) else {
                continue;
            };
            d.state[k] = Some(s);
            d.set_handlers(model, k, s);
        }
        d
    }

    /// The parts with health of their own.
    pub fn with_health(&self) -> usize {
        self.health.iter().flatten().count()
    }

    /// Part `k`'s own `ModifierKey` (a weak point's), if it has one.
    pub fn key(&self, k: usize) -> Option<u32> {
        self.keys.get(k).copied().flatten()
    }

    /// The parts with a `ModifierKey` of their own.
    pub fn with_key(&self) -> usize {
        self.keys.iter().flatten().count()
    }

    fn set_handlers(&mut self, model: &DamageModel, k: usize, s: usize) {
        let Some(acts) = model.states.machines[k].acts.get(s) else {
            return;
        };
        for &(msg, to) in &acts.on {
            let h = &mut self.handlers[k];
            h.retain(|&(m, _)| m != msg);
            if to != 0 {
                h.push((msg, to));
            }
        }
    }

    /// Part `k` takes message `msg`: to the state its handler names.
    pub fn send(&mut self, model: &DamageModel, k: usize, msg: u32) {
        self.send_chained(model, k, msg, 0);
    }

    fn send_chained(&mut self, model: &DamageModel, k: usize, msg: u32, depth: u32) {
        let Some(&(_, to)) = self
            .handlers
            .get(k)
            .and_then(|h| h.iter().find(|x| x.0 == msg))
        else {
            return;
        };
        let m = &model.states.machines[k];
        if let Some(s) = m.states.iter().position(|x| x.0 == to) {
            self.enter(model, k, s, depth + 1);
        }
    }

    /// Part `k` enters its state `s`: its handlers are set, the parts it
    /// throws off are noted, the states it sets on parts are entered and
    /// the damage it passes on is dealt.
    fn enter(&mut self, model: &DamageModel, k: usize, s: usize, depth: u32) {
        if depth > CHAIN {
            return;
        }
        if let Some(left) = self.state[k].and_then(|p| model.states.machines[k].acts.get(p)) {
            self.running
                .retain(|&(j, fx, _)| j != k || !left.stops.contains(&fx));
        }
        self.state[k] = Some(s);
        self.entered += 1;
        self.set_handlers(model, k, s);
        let Some(acts) = model.states.machines[k].acts.get(s).cloned() else {
            return;
        };
        for (fx, at) in acts.emitters {
            if let Ok(n) = at
                && !self.running.contains(&(k, fx, n))
            {
                self.running.push((k, fx, n));
            }
        }
        for node in acts.detaches {
            let centre =
                model.boxes[k].map_or_else(|| model.node_at(node), |(lo, hi)| (lo + hi) * 0.5);
            self.detached.push(Detached {
                part: k,
                node,
                centre,
            });
        }
        for (to, node) in acts.sets {
            let Some(j) = model
                .states
                .machines
                .iter()
                .position(|m| m.node == Some(node))
            else {
                continue;
            };
            if let Some(t) = model.states.machines[j]
                .states
                .iter()
                .position(|x| x.0 == to)
            {
                self.enter(model, j, t, depth + 1);
            }
        }
        for (amount, part) in acts.sends {
            if let Some(j) = model.part_named(part) {
                self.hurt_chained(model, j, amount, depth + 1);
            }
        }
    }

    /// Part `k` takes `damage` (times its scale): damaged at its
    /// damaged-at health, destroyed at none.
    pub fn hurt(&mut self, model: &DamageModel, k: usize, damage: f32) {
        self.hurt_chained(model, k, damage, 0);
    }

    fn hurt_chained(&mut self, model: &DamageModel, k: usize, damage: f32, depth: u32) {
        let Some(Some((left, rec))) = self.health.get_mut(k) else {
            return;
        };
        *left -= damage * rec.scale;
        let (left, rec) = (*left, *rec);
        if rec.damaged_at > 0.0 && left <= rec.damaged_at && !self.sent[k][0] {
            self.sent[k][0] = true;
            self.send_chained(model, k, DAMAGE_MSG, depth);
        }
        if left <= 0.0 && !self.sent[k][1] {
            self.sent[k][1] = true;
            self.send_chained(model, k, DESTROY_MSG, depth);
        }
    }

    /// A hit of `damage` at `at` (model space): the part it strikes
    /// takes it. The part struck, if any.
    pub fn hit(&mut self, model: &DamageModel, at: Vec3, damage: f32) -> Option<usize> {
        let k = model.part_at(at, HIT_SLACK)?;
        if Some(k) == self.body {
            return Some(k);
        }
        self.hurt(model, k, damage);
        Some(k)
    }

    /// A blast at `at` (model space) reaching `reach` metres: every part
    /// but the body within reach takes `damage_at` its distance from the
    /// blast's heart (an explosion template's own fall from `Damage` to
    /// `MinDamage`).
    pub fn blast(
        &mut self,
        model: &DamageModel,
        at: Vec3,
        reach: f32,
        damage_at: impl Fn(f32) -> f32,
    ) {
        for k in 0..model.boxes.len() {
            let Some((lo, hi)) = model.boxes[k] else {
                continue;
            };
            if Some(k) == self.body {
                continue;
            }
            let d = (lo - at).max(at - hi).max(Vec3::ZERO).length();
            if d < reach {
                self.hurt(model, k, damage_at(d));
            }
        }
    }

    /// The body is damaged (the vehicle burns down from here on).
    pub fn body_damaged(&mut self, model: &DamageModel) {
        if let Some(k) = self.body
            && !self.sent[k][0]
        {
            self.sent[k][0] = true;
            self.send(model, k, DAMAGE_MSG);
        }
    }

    /// The body is destroyed (the vehicle is wrecked).
    pub fn body_destroyed(&mut self, model: &DamageModel) {
        if let Some(k) = self.body
            && !self.sent[k][1]
        {
            self.sent[k][1] = true;
            self.send(model, k, DESTROY_MSG);
        }
    }

    /// The parts thrown off since last asked.
    pub fn take_detached(&mut self) -> Vec<Detached> {
        std::mem::take(&mut self.detached)
    }

    /// The effects the parts' states keep going: (part, effect name hash,
    /// model-space place), but those at a locator the model lacks.
    pub fn emitters(&self, model: &DamageModel) -> Vec<(usize, u32, Vec3)> {
        self.running
            .iter()
            .map(|&(k, fx, n)| (k, fx, model.node_at(n)))
            .collect()
    }

    /// Per node, whether it is hidden with the parts as they stand.
    pub fn hidden(&self, model: &DamageModel) -> Vec<bool> {
        model
            .states
            .hidden_in(&model.nodes, |k, _| self.state.get(k).copied().flatten())
    }

    /// Its parts as a vehicle loaded wrecked has them: each where its
    /// `InitDestroyedState` puts it (else its wrecked state), nothing
    /// burning or smoking, and no message taken any more.
    pub fn start_destroyed(&mut self, model: &DamageModel) {
        for (k, m) in model.states.machines.iter().enumerate() {
            let state = |name: u32| m.states.iter().position(|s| s.0 == name);
            if let Some(s) = m.starts[1]
                .and_then(state)
                .or_else(|| state(m2_assets::model::STATE_WRECKED))
            {
                self.state[k] = Some(s);
            }
            self.sent[k] = [true; 2];
            self.handlers[k].clear();
        }
        self.running.clear();
        self.detached.clear();
    }

    /// Whether every part stands as it started.
    pub fn untouched(&self) -> bool {
        self.entered == 0
    }
}

/// How far from a part's box a hit still strikes it, metres (invented).
pub const HIT_SLACK: f32 = 0.15;

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::model::{PartMachine, StateActs};

    fn node(name: u32, parent: Option<u16>, lo: [f32; 3], hi: [f32; 3]) -> Node {
        Node {
            name_hash: name,
            flags: 0,
            first_child: None,
            parent,
            next_sibling: None,
            unk_0x0c: 0,
            local: Default::default(),
            inverse_bind: Default::default(),
            bounds_min: [lo[0], lo[1], lo[2], 1.0],
            bounds_max: [hi[0], hi[1], hi[2], 1.0],
        }
    }

    const PRISTINE: u32 = m2_assets::model::STATE_INTACT;
    const DENTED: u32 = 0x381b_e6a4;
    const THROWN: u32 = 0xce60_3754;
    const GONE: u32 = 0xca26_1e5b;
    const FIRE: u32 = 0x1111_1111;
    const SMOKE: u32 = 0x2222_2222;
    const WRECKING: u32 = 0x9279_1ebb;

    /// A car as the Valiant's machine has it: the root's body part (node
    /// 1, showing body 2, its damaged state starting a fire at 2) and a
    /// door part (node 3: whole door 4, dented door 5) that dents, then
    /// comes off.
    fn car() -> DamageModel {
        let nodes = vec![
            node(100, None, [0.0; 3], [0.0; 3]),
            node(101, Some(0), [0.0; 3], [0.0; 3]),
            node(102, Some(1), [-1.0, 0.0, -2.0], [1.0, 1.5, 2.0]),
            node(103, Some(0), [0.0; 3], [0.0; 3]),
            node(104, Some(3), [0.9, 0.2, 0.0], [1.0, 1.0, 1.2]),
            node(105, Some(3), [0.9, 0.2, 0.0], [1.0, 1.0, 1.2]),
        ];
        let acts = |on: Vec<(u32, u32)>| StateActs {
            on,
            ..StateActs::default()
        };
        let body = PartMachine {
            node: Some(1),
            states: vec![
                (PRISTINE, vec![(2, true)]),
                (0x1d55_75a1, vec![(2, true)]),
                (WRECKING, vec![]),
                (m2_assets::model::STATE_WRECKED, vec![]),
            ],
            starts: [None; 2],
            acts: vec![
                acts(vec![(DAMAGE_MSG, 0x1d55_75a1), (DESTROY_MSG, WRECKING)]),
                StateActs {
                    on: vec![(DAMAGE_MSG, 0)],
                    emitters: vec![(FIRE, Ok(2)), (FIRE, Err(0x7eb2_c415))],
                    stops: vec![FIRE],
                    ..StateActs::default()
                },
                // On its way to the wreck: its smoke starts, and runs on in
                // the wreck's state.
                StateActs {
                    emitters: vec![(SMOKE, Ok(1))],
                    sets: vec![(m2_assets::model::STATE_WRECKED, 1)],
                    ..StateActs::default()
                },
                StateActs::default(),
            ],
        };
        let door = PartMachine {
            node: Some(3),
            states: vec![
                (PRISTINE, vec![(4, true)]),
                (DENTED, vec![(5, true)]),
                (THROWN, vec![]),
                (GONE, vec![]),
            ],
            starts: [None; 2],
            acts: vec![
                acts(vec![(DAMAGE_MSG, DENTED), (DESTROY_MSG, THROWN)]),
                acts(vec![(DAMAGE_MSG, 0)]),
                StateActs {
                    on: vec![(DESTROY_MSG, 0)],
                    detaches: vec![5],
                    sets: vec![(GONE, 3)],
                    ..StateActs::default()
                },
                StateActs::default(),
            ],
        };
        let states = PartStates {
            switchable: vec![2, 4, 5],
            machines: vec![body, door],
            ..PartStates::default()
        };
        let rest = vec![Mat4::IDENTITY; nodes.len()];
        let mut m = DamageModel {
            states,
            nodes,
            rest,
            boxes: Vec::new(),
        };
        m.boxes = vec![
            Some((Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 1.5, 2.0))),
            Some((Vec3::new(0.9, 0.2, 0.0), Vec3::new(1.0, 1.0, 1.2))),
        ];
        m
    }

    fn healths() -> PartHealths {
        PartHealths {
            parts: vec![
                NodeHealth {
                    part: 103,
                    health: 30.0,
                    scale: 1.0,
                    damaged_at: 15.0,
                },
                NodeHealth {
                    part: 101,
                    health: 1000.0,
                    scale: 1.0,
                    damaged_at: 0.0,
                },
            ],
            burn_part: Some(101),
            keys: Vec::new(),
            debris: Vec::new(),
        }
    }

    #[test]
    fn a_door_dents_at_its_threshold_then_comes_off() {
        let m = car();
        let mut d = PartDamage::new(&m, &healths());
        assert_eq!(d.body, Some(0));
        assert_eq!(d.with_health(), 2);
        // Intact: the whole door shows, the dented one doesn't.
        assert_eq!(d.hidden(&m), [false, false, false, false, false, true]);
        // A rifle round's 9 at the door: 21 left, still whole.
        assert_eq!(d.hit(&m, Vec3::new(0.95, 0.5, 0.6), 9.0), Some(1));
        assert!(d.untouched());
        // At 12 it dents (its threshold 15).
        d.hit(&m, Vec3::new(0.95, 0.5, 0.6), 9.0);
        assert_eq!(d.state[1], Some(1));
        assert_eq!(d.hidden(&m), [false, false, false, false, true, false]);
        // The dented state drops `DamageMsg`, but `DestroyMsg` still
        // throws it off: then nothing of it shows.
        d.hit(&m, Vec3::new(0.95, 0.5, 0.6), 9.0);
        d.hit(&m, Vec3::new(0.95, 0.5, 0.6), 9.0);
        assert_eq!(d.state[1], Some(3));
        let thrown = d.take_detached();
        assert_eq!(thrown.len(), 1);
        assert_eq!(thrown[0].node, 5);
        assert!(d.take_detached().is_empty());
        assert_eq!(d.hidden(&m), [false, false, false, false, true, true]);
    }

    #[test]
    fn hits_on_the_body_alone_leave_it_and_its_fire_waits_for_the_burn() {
        let m = car();
        let mut d = PartDamage::new(&m, &healths());
        // The roof: only the body's box holds it.
        assert_eq!(d.hit(&m, Vec3::new(0.0, 1.5, -1.0), 50.0), Some(0));
        assert!(d.untouched());
        assert!(d.emitters(&m).is_empty());
        d.body_damaged(&m);
        // The fire at node 2; the one at a locator the model lacks is
        // left out.
        assert_eq!(d.emitters(&m), [(0, FIRE, Vec3::ZERO)]);
        d.body_damaged(&m);
        assert_eq!(d.entered, 1);
        // Wrecked: the fire stops as it leaves the burning state; the
        // smoke started on the way runs on in the wreck's.
        d.body_destroyed(&m);
        assert_eq!(d.state[0], Some(3));
        assert_eq!(d.emitters(&m), [(0, SMOKE, Vec3::ZERO)]);
    }

    #[test]
    fn a_blast_reaches_the_parts_in_its_reach() {
        let m = car();
        let mut d = PartDamage::new(&m, &healths());
        d.blast(&m, Vec3::new(3.0, 0.5, 0.5), 5.0, |_| 200.0);
        assert_eq!(d.state[1], Some(3));
        let mut far = PartDamage::new(&m, &healths());
        far.blast(&m, Vec3::new(9.0, 0.5, 0.5), 5.0, |_| 200.0);
        assert!(far.untouched());
    }

    #[test]
    fn a_blast_hurts_each_part_by_its_own_distance() {
        let m = car();
        // The door's box is 2 m from the heart: a blast falling from 20
        // to 10 over 4 m gives it 15 of its 30, and it dents at 15.
        let fall = |d: f32| 20.0 - 10.0 * d / 4.0;
        let mut d = PartDamage::new(&m, &healths());
        d.blast(&m, Vec3::new(3.0, 0.5, 0.5), 4.0, fall);
        assert_eq!(d.state[1], Some(1));
        // The reach's edge's 10 where it stands leaves it whole.
        let mut edge = PartDamage::new(&m, &healths());
        edge.blast(&m, Vec3::new(3.0, 0.5, 0.5), 4.0, |_| 10.0);
        assert!(edge.untouched());
    }

    #[test]
    fn a_part_with_its_own_key_is_a_weak_point() {
        let m = car();
        let weak = m2_assets::name_hash("WeakPointTank");
        let mut h = healths();
        // The door's node, and a node the model doesn't have.
        h.keys = vec![(103, weak), (999, weak)];
        let d = PartDamage::new(&m, &h);
        assert_eq!(d.with_key(), 1);
        assert_eq!(d.key(1), Some(weak));
        assert_eq!(d.key(0), None);
        assert_eq!(d.key(7), None);
    }
}
