//! The parts a model's state machine sends off as objects of their own
//! (`CreateObject` naming `DebrisTemplate`: a burning palm's leaves): for
//! each, the node it goes off from, the nodes that go with it (it and
//! everything under it), where they sit in the model at rest and the box
//! round their geometry.
//!
//! **From the 360 code:** a state's `CreateObject` (verb `0xC6E8AFA8`, run
//! on entering the state by 0x82230030 from the verb table 0x8222E480)
//! takes the part's node, the state its new object starts in and the
//! template field it names (`DebrisTemplate`, the object's
//! `OSMParameter` of that name, component 0x83655494); it spawns that
//! template (0x82408B88, 0x82409B38) at the node's place in the world
//! (0x82426F40), drawn as the node's own piece of the model (0x82429658),
//! its part machine starting in the state named, and ties it to the
//! object it left (0x8222FF58).
//!
//! **Inferred:** that the piece is the node and every node under it (the
//! leaves' geometry hangs under their node), and that it leaves the
//! object it came from at rest (no push is given it).

use m2_assets::model::Model;
use m2_math::glam::{Mat4, Vec3};

/// One part that goes off.
#[derive(Clone, Debug, PartialEq)]
pub struct NodePiece {
    /// Its node's name hash: the part machine's name
    /// ([`crate::part_runner::Change::machine`]).
    pub name: u32,
    pub node: u16,
    /// The template field naming what it goes as (`DebrisTemplate`): the
    /// object's `OSMParameter` of that name ([`params`]).
    pub param: u32,
    /// The node it is placed at instead of its own (the `CreateObject`'s
    /// second node, when not 0: 0x82230030 looks that one up first).
    pub at: Option<u16>,
    /// Per node of the model, whether it goes with the piece.
    pub nodes: Vec<bool>,
    /// The box round the piece's geometry, model space (`None`: nothing
    /// of its own to show).
    pub bounds: Option<(Vec3, Vec3)>,
}

/// A model's parts that go off ([`NodePiece`]), and each node's place in
/// the model at rest.
#[derive(Clone, Debug, PartialEq)]
pub struct NodePieces {
    pub pieces: Vec<NodePiece>,
    pub rest: Vec<Mat4>,
}

impl NodePieces {
    /// `model`'s, if its states send any part off.
    pub fn of(model: &Model<'_>) -> Option<Self> {
        let states = model.states.as_ref()?;
        // Each part sent off once, as the first `CreateObject` naming it.
        let mut sent: Vec<m2_assets::model::Create> = Vec::new();
        for c in states
            .machines
            .iter()
            .flat_map(|m| &m.acts)
            .flat_map(|a| &a.creates)
        {
            if !sent.iter().any(|s| s.node == c.node) {
                sent.push(*c);
            }
        }
        sent.sort_unstable_by_key(|c| c.node);
        if sent.is_empty() {
            return None;
        }
        let parents: Vec<Option<u16>> = model.nodes.iter().map(|n| n.parent).collect();
        let rest: Vec<Mat4> = (0..model.nodes.len())
            .map(|i| model.node_to_model(i as u16).unwrap_or(Mat4::IDENTITY))
            .collect();
        let boxes: Vec<Option<(Vec3, Vec3)>> = model
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let (b, t) = (n.bounds_min, n.bounds_max);
                let (lo, hi) = (Vec3::new(b[0], b[1], b[2]), Vec3::new(t[0], t[1], t[2]));
                ((hi - lo).length_squared() > 1e-6).then(|| world_box(rest[i], lo, hi))
            })
            .collect();
        let pieces = sent
            .into_iter()
            .filter(|c| usize::from(c.node) < model.nodes.len())
            .map(|c| {
                let node = c.node;
                let nodes = subtree(&parents, node);
                NodePiece {
                    name: model.nodes[usize::from(node)].name_hash,
                    node,
                    param: c.param,
                    at: c
                        .at
                        .filter(|&a| a != node && usize::from(a) < model.nodes.len()),
                    bounds: union(
                        nodes
                            .iter()
                            .zip(&boxes)
                            .filter(|(k, _)| **k)
                            .filter_map(|(_, b)| *b),
                    ),
                    nodes,
                }
            })
            .collect();
        Some(Self { pieces, rest })
    }

    /// Where piece `k`'s body starts, the object placed at `world`, from
    /// where it would start on its own node (`start`, world): moved by
    /// its second node's place against its own when it has one (the 360
    /// places the new object at the parent's world transform times that
    /// node's, 0x823F8370 x 0x82426F40).
    pub fn placed(&self, k: usize, world: Mat4, start: Mat4) -> Mat4 {
        let Some(p) = self.pieces.get(k) else {
            return start;
        };
        match p.at {
            Some(at) => {
                let (own, there) = (self.rest[usize::from(p.node)], self.rest[usize::from(at)]);
                world * there * own.inverse() * world.inverse() * start
            }
            None => start,
        }
    }

    /// The piece that goes off from the node named `name`.
    pub fn by_name(&self, name: u32) -> Option<&NodePiece> {
        self.pieces.iter().find(|p| p.name == name)
    }
}

/// Every template's `OSMParameter` records whose value names a template,
/// by its id: (the parameter's `Name` hash, the id of the template it
/// names). What a part sent off goes as is its object's record named by
/// the `CreateObject` ([`NodePiece::param`]).
pub fn params(
    db: &m2_assets::Layer<'_>,
    names: &std::collections::HashMap<u32, &str>,
) -> std::collections::HashMap<u32, Vec<(u32, u32)>> {
    let by_hash = by_hash(names);
    let osm = crate::tuning::Table::new(db, "OSMParameter");
    let name = m2_assets::name_hash("Name");
    osm.templates()
        .filter_map(|id| {
            let named: Vec<(u32, u32)> = osm
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let t = *by_hash.get(&osm.word(r, crate::durability::OSM_VALUE)?)?;
                    Some((osm.word(r, name)?, t))
                })
                .collect();
            (!named.is_empty()).then_some((id, named))
        })
        .collect()
}

/// Template ids by their name's hash.
pub fn by_hash(
    names: &std::collections::HashMap<u32, &str>,
) -> std::collections::HashMap<u32, u32> {
    names
        .iter()
        .map(|(&id, n)| (m2_assets::name_hash(n), id))
        .collect()
}

/// The template a part sent off from an object of template `object`
/// goes as, its `CreateObject` naming field `param`: the object's
/// `OSMParameter` of that name, else the template `param` itself names
/// (0x82230030: the parameter's value when the object has it, the name
/// as it is when not, resolved by 0x82408B88).
pub fn goes_as(
    params: &std::collections::HashMap<u32, Vec<(u32, u32)>>,
    by_hash: &std::collections::HashMap<u32, u32>,
    object: u32,
    param: u32,
) -> Option<u32> {
    params
        .get(&object)
        .and_then(|v| v.iter().find(|p| p.0 == param))
        .map(|p| p.1)
        .or_else(|| by_hash.get(&param).copied())
}

/// Per node, whether it is `top` or under it (`parents`: each node's).
pub fn subtree(parents: &[Option<u16>], top: u16) -> Vec<bool> {
    (0..parents.len())
        .map(|i| {
            let mut at = Some(i as u16);
            for _ in 0..=parents.len() {
                match at {
                    Some(n) if n == top => return true,
                    Some(n) => at = parents.get(usize::from(n)).copied().flatten(),
                    None => return false,
                }
            }
            false
        })
        .collect()
}

/// The box round a node's box `lo` to `hi` placed by `at`.
fn world_box(at: Mat4, lo: Vec3, hi: Vec3) -> (Vec3, Vec3) {
    let mut b: Option<(Vec3, Vec3)> = None;
    for k in 0..8 {
        let c = Vec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        );
        let p = at.transform_point3(c);
        b = Some(b.map_or((p, p), |(a, z)| (a.min(p), z.max(p))));
    }
    b.unwrap_or((lo, hi))
}

fn union(boxes: impl Iterator<Item = (Vec3, Vec3)>) -> Option<(Vec3, Vec3)> {
    boxes.reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piece_takes_its_node_and_everything_under_it() {
        // 0 the trunk; 1 the crown under it; 2 and 3 leaves under the
        // crown; 4 a root beside.
        let parents = [None, Some(0), Some(1), Some(1), Some(0)];
        assert_eq!(subtree(&parents, 1), [false, true, true, true, false]);
        assert_eq!(subtree(&parents, 4), [false, false, false, false, true]);
        assert_eq!(subtree(&parents, 0), [true; 5]);
    }

    /// A part a delayed state sends off leaves when that state comes in,
    /// another sent off at once leaves at once (a model whose states give
    /// the leaves their own delays; the palms' data has none).
    #[test]
    fn a_part_a_delayed_state_sends_off_leaves_later() {
        use m2_assets::model::{DESTROY_MSG, PartMachine, PartStates, STATE_INTACT, StateActs};
        // The trunk (node 0) on DestroyMsg goes to 0xA, which sends leaf
        // 1 off at once and sets 0xB 2 to 2 s later, which sends leaf 2 off.
        let (gone, later, off) = (0xa, 0xb, 0xc);
        let trunk = PartMachine {
            node: Some(0),
            states: vec![(STATE_INTACT, vec![]), (gone, vec![]), (later, vec![])],
            starts: [Some(STATE_INTACT), None],
            acts: vec![
                StateActs {
                    on: vec![(DESTROY_MSG, gone)],
                    ..Default::default()
                },
                StateActs {
                    creates: vec![m2_assets::model::Create {
                        state: off,
                        node: 1,
                        ..Default::default()
                    }],
                    delayed: vec![(later, 0, 2.0, 2.0)],
                    ..Default::default()
                },
                StateActs {
                    creates: vec![m2_assets::model::Create {
                        state: off,
                        node: 2,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        };
        let leaf = |n: u16| PartMachine {
            node: Some(n),
            states: vec![(STATE_INTACT, vec![]), (off, vec![])],
            starts: [Some(STATE_INTACT), None],
            acts: vec![StateActs::default(), StateActs::default()],
        };
        let states = PartStates {
            machines: vec![trunk, leaf(1), leaf(2)],
            ..Default::default()
        };
        let mut runner = crate::part_runner::Runner::new(&states, |n| 100 + u32::from(n), 1);
        runner.deliver(DESTROY_MSG);
        let sent = |r: &mut crate::part_runner::Runner| -> Vec<u32> {
            r.take_changes()
                .into_iter()
                .filter(|c| c.sent_off)
                .map(|c| c.machine)
                .collect()
        };
        assert_eq!(sent(&mut runner), [101]);
        runner.advance(1.9);
        assert!(sent(&mut runner).is_empty());
        runner.advance(0.2);
        assert_eq!(sent(&mut runner), [102]);
    }

    #[test]
    fn a_piece_goes_as_its_objects_parameter_else_the_name_itself() {
        use std::collections::HashMap;
        let debris = m2_assets::model::DEBRIS_TEMPLATE;
        // Object template 7's DebrisTemplate names template 40.
        let params = HashMap::from([(7, vec![(0x1234, 39), (debris, 40)])]);
        let by_hash = HashMap::from([(debris, 99)]);
        assert_eq!(goes_as(&params, &by_hash, 7, debris), Some(40));
        // One without it: the field's name as a template's.
        assert_eq!(goes_as(&params, &by_hash, 8, debris), Some(99));
        assert_eq!(goes_as(&params, &by_hash, 8, 0x5555), None);
    }

    #[test]
    fn a_piece_with_a_second_node_starts_at_that_node() {
        let rest = vec![
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::Y),
            Mat4::from_translation(Vec3::new(3.0, 1.0, 0.0)),
        ];
        let piece = |at| NodePiece {
            name: 1,
            node: 1,
            param: 0,
            at,
            nodes: vec![false, true, false],
            bounds: None,
        };
        let world = Mat4::from_translation(Vec3::Z * 10.0);
        let start = world * Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0));
        let p = NodePieces {
            pieces: vec![piece(None), piece(Some(2))],
            rest,
        };
        assert_eq!(p.placed(0, world, start), start);
        let moved = p.placed(1, world, start).w_axis.truncate();
        assert!(moved.distance(Vec3::new(3.0, 1.5, 10.0)) < 1e-5, "{moved}");
    }

    #[test]
    fn its_box_holds_its_nodes_boxes_where_they_sit() {
        let up = Mat4::from_translation(Vec3::Y * 5.0);
        let b = world_box(up, Vec3::splat(-1.0), Vec3::splat(1.0));
        assert_eq!(b, (Vec3::new(-1.0, 4.0, -1.0), Vec3::new(1.0, 6.0, 1.0)));
        let u = union([b, (Vec3::ZERO, Vec3::splat(8.0))].into_iter());
        assert_eq!(u, Some((Vec3::new(-1.0, 0.0, -1.0), Vec3::splat(8.0))));
    }
}
