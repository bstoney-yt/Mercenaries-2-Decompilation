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
        let mut sent: Vec<u16> = states
            .machines
            .iter()
            .flat_map(|m| &m.acts)
            .flat_map(|a| a.creates.iter().map(|&(_, node)| node))
            .collect();
        sent.sort_unstable();
        sent.dedup();
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
            .filter(|&n| usize::from(n) < model.nodes.len())
            .map(|node| {
                let nodes = subtree(&parents, node);
                NodePiece {
                    name: model.nodes[usize::from(node)].name_hash,
                    node,
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

    /// The piece that goes off from the node named `name`.
    pub fn by_name(&self, name: u32) -> Option<&NodePiece> {
        self.pieces.iter().find(|p| p.name == name)
    }
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

    #[test]
    fn its_box_holds_its_nodes_boxes_where_they_sit() {
        let up = Mat4::from_translation(Vec3::Y * 5.0);
        let b = world_box(up, Vec3::splat(-1.0), Vec3::splat(1.0));
        assert_eq!(b, (Vec3::new(-1.0, 4.0, -1.0), Vec3::new(1.0, 6.0, 1.0)));
        let u = union([b, (Vec3::ZERO, Vec3::splat(8.0))].into_iter());
        assert_eq!(u, Some((Vec3::new(-1.0, 0.0, -1.0), Vec3::splat(8.0))));
    }
}
