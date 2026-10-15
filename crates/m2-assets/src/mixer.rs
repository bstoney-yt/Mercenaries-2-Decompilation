//! Model mixers (`MIXR`): the interchangeable parts a person model carries
//! (heads, bodies, headgear, kit) and which of them each kind of person may
//! wear, so that soldiers of one template don't all look alike.
//!
//! A model's `MIXR` container holds:
//!
//! ```text
//! INFO  u16 version (1), u16 part count n, u16 profile count
//! NAME  u32 * n          each part's name hash: a node of the model's
//!                        `HIER`, or a name no node has (an empty pick:
//!                        wearing nothing in that place)
//! EXMX  u8 * n * n       row-major: 1 = the two parts can be worn
//!                        together, 0 = they can't (one head at a time;
//!                        the diagonal is 0)
//! PROF  (u32 + u8 * n) * profile count
//!                        a profile's name hash (what a template's
//!                        `ModelMixerProfile` names: `VZ Soldier`, `VZ
//!                        Officer`), then per part 1 = it may wear it
//! ```
//!
//! The part nodes are the model's switchable nodes (its `STAM` lists the
//! same nodes, each with two states and no start state), so each starts
//! hidden; a look shows the parts picked and leaves the rest hidden.
//!
//! # Provenance
//!
//! From samples: read off the 34 of 3,007 models that carry a `MIXR`
//! (round variety20), the VZ soldier's (`09139bcd`, 18 parts, 6 profiles)
//! first. All 34 are version 1, and the part count fits `NAME`, `EXMX` and
//! `PROF` in every one. Each has exactly one name no node has, always the
//! last. `EXMX` is symmetric in 23; in the other 11 a few pairs go together
//! one way only, mostly against the empty pick, so two parts go together
//! here only when both ways say so (inferred). 17 of the soldier's 18
//! names are nodes, whose segments are the model's only geometry.
//! The meanings of 1 and 0 in `EXMX` and `PROF` are inferred from the
//! groups they make (five parts that exclude each other and that every
//! profile allows one or more of: the heads). How the game picks among the
//! parts a profile allows is not in the data: [`Mixer::pick`] takes a
//! random order and wears each allowed part that fits with those already
//! worn (invented).

use crate::bytes::Reader;
use crate::model::Node;
use crate::ucfx::Ucfx;

/// One part a mixer can put on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MixerPart {
    pub name_hash: u32,
    /// The model node that holds its geometry; `None` for a name no node
    /// has (an empty pick).
    pub node: Option<u16>,
}

/// A model's mixer: its parts, which go together, and its profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mixer {
    pub version: u16,
    pub parts: Vec<MixerPart>,
    /// `parts.len()` squared, row-major: whether two parts go together.
    together: Vec<bool>,
    /// (profile name hash, per part whether it may be worn).
    pub profiles: Vec<(u32, Vec<bool>)>,
}

impl Mixer {
    /// Reads the `MIXR` container at section `mixr` of `u`, naming parts by
    /// `nodes`. `None` when a piece is missing or the sizes don't agree.
    pub fn parse(u: &Ucfx<'_>, mixr: usize, nodes: &[Node]) -> Option<Self> {
        let mut info = None;
        let mut names = None;
        let mut exmx = None;
        let mut prof = None;
        for c in u.children(Some(mixr)) {
            let s = &u.sections()[c];
            match &s.fourcc {
                b"INFO" => info = s.data,
                b"NAME" => names = s.data,
                b"EXMX" => exmx = s.data,
                b"PROF" => prof = s.data,
                _ => {}
            }
        }
        let mut r = Reader::at(info?, 0);
        let version = r.u16().ok()?;
        let n = usize::from(r.u16().ok()?);
        let profiles = usize::from(r.u16().ok()?);
        let (names, exmx, prof) = (names?, exmx?, prof?);
        if names.len() != 4 * n || exmx.len() != n * n || prof.len() != profiles * (4 + n) {
            return None;
        }
        let parts = names
            .chunks_exact(4)
            .map(|c| {
                let name_hash = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                MixerPart {
                    name_hash,
                    node: nodes
                        .iter()
                        .position(|x| x.name_hash == name_hash)
                        .and_then(|i| u16::try_from(i).ok()),
                }
            })
            .collect();
        let profiles = prof
            .chunks_exact(4 + n)
            .map(|c| {
                let hash = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                (hash, c[4..].iter().map(|&b| b != 0).collect())
            })
            .collect();
        Some(Self {
            version,
            parts,
            together: exmx.iter().map(|&b| b != 0).collect(),
            profiles,
        })
    }

    /// Whether `EXMX` says parts `a` and `b` can be worn together (row
    /// `a`; [`pick`](Self::pick) and [`looks`](Self::looks) want both
    /// ways).
    pub fn together(&self, a: usize, b: usize) -> bool {
        self.together
            .get(a * self.parts.len() + b)
            .copied()
            .unwrap_or(false)
    }

    /// Whether every pair goes together both ways round.
    pub fn is_symmetric(&self) -> bool {
        let n = self.parts.len();
        (0..n).all(|a| (0..n).all(|b| self.together(a, b) == self.together(b, a)))
    }

    /// The parts profile `profile` may wear; every part for a profile the
    /// mixer doesn't have (a template naming none, or a model standing in
    /// for its template's own).
    pub fn allowed(&self, profile: Option<u32>) -> Vec<bool> {
        profile
            .and_then(|h| self.profiles.iter().find(|p| p.0 == h))
            .map_or_else(|| vec![true; self.parts.len()], |p| p.1.clone())
    }

    /// A look: of the parts `allowed` marks, in an order `roll` shuffles
    /// (`roll(k)` gives a number below `k`), each that goes with those
    /// already picked. The picks, by index into [`parts`](Self::parts), in
    /// part order.
    pub fn pick(&self, allowed: &[bool], mut roll: impl FnMut(usize) -> usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.parts.len())
            .filter(|&i| allowed.get(i).copied().unwrap_or(false))
            .collect();
        for k in (1..order.len()).rev() {
            order.swap(k, roll(k + 1).min(k));
        }
        let mut picked: Vec<usize> = Vec::new();
        for i in order {
            if picked
                .iter()
                .all(|&j| self.together(i, j) && self.together(j, i))
            {
                picked.push(i);
            }
        }
        picked.sort_unstable();
        picked
    }

    /// How many different looks [`pick`](Self::pick) can give from the
    /// parts `allowed` marks: the sets of parts that all go together to
    /// which no other allowed part can be added. Counting stops at `cap`.
    pub fn looks(&self, allowed: &[bool], cap: usize) -> usize {
        let candidates: Vec<usize> = (0..self.parts.len())
            .filter(|&i| allowed.get(i).copied().unwrap_or(false))
            .collect();
        let mut count = 0;
        self.maximal(Vec::new(), candidates, Vec::new(), cap, &mut count);
        count
    }

    /// Bron-Kerbosch over the "go together" graph: counts maximal sets.
    fn maximal(
        &self,
        r: Vec<usize>,
        mut p: Vec<usize>,
        mut x: Vec<usize>,
        cap: usize,
        count: &mut usize,
    ) {
        if *count >= cap {
            return;
        }
        if p.is_empty() && x.is_empty() {
            *count += 1;
            return;
        }
        while let Some(v) = p.pop() {
            let near = |s: &[usize]| -> Vec<usize> {
                s.iter()
                    .copied()
                    .filter(|&u| self.together(u, v) && self.together(v, u))
                    .collect()
            };
            let mut r2 = r.clone();
            r2.push(v);
            self.maximal(r2, near(&p), near(&x), cap, count);
            x.push(v);
        }
    }

    /// Per node of `nodes`, whether a look of `picked` hides it: it is, or
    /// hangs under, a part's node that wasn't picked.
    pub fn hidden(&self, picked: &[usize], nodes: &[Node]) -> Vec<bool> {
        let mut part_of = vec![None; nodes.len()];
        for (i, p) in self.parts.iter().enumerate() {
            if let Some(slot) = p.node.and_then(|n| part_of.get_mut(usize::from(n))) {
                *slot = Some(i);
            }
        }
        (0..nodes.len())
            .map(|i| {
                let mut at = Some(i);
                for _ in 0..=nodes.len() {
                    let Some(n) = at else { break };
                    if let Some(part) = part_of[n] {
                        return !picked.contains(&part);
                    }
                    at = nodes[n].parent.map(usize::from);
                }
                false
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn node(name_hash: u32, parent: Option<u16>) -> Node {
        Node {
            name_hash,
            flags: 0,
            first_child: None,
            parent,
            next_sibling: None,
            unk_0x0c: 0,
            local: Default::default(),
            inverse_bind: Default::default(),
            bounds_min: [0.0; 4],
            bounds_max: [0.0; 4],
        }
    }

    /// Two heads (0x10, 0x11) that exclude each other, a cap (0x20) and an
    /// empty pick (0x30, no node) that exclude each other, and a vest
    /// (0x40) that goes with everything. Profile 0xA wears either head and
    /// either cap; profile 0xB only head 0x11, no cap, and the vest.
    fn mixer_asset() -> Vec<u8> {
        let names = [0x10u32, 0x11, 0x20, 0x30, 0x40];
        let n = names.len();
        let mut exmx = vec![1u8; n * n];
        for (a, b) in [(0, 0), (1, 1), (0, 1), (2, 2), (3, 3), (2, 3), (4, 4)] {
            exmx[a * n + b] = 0;
            exmx[b * n + a] = 0;
        }
        let mut info = 1u16.to_le_bytes().to_vec();
        info.extend((n as u16).to_le_bytes());
        info.extend(2u16.to_le_bytes());
        let mut prof = Vec::new();
        for (h, flags) in [(0xAu32, [1u8, 1, 1, 1, 0]), (0xB, [0, 1, 0, 1, 1])] {
            prof.extend(h.to_le_bytes());
            prof.extend(flags);
        }
        ucfx(&[UcfxNode::Container(
            *b"MIXR",
            vec![
                UcfxNode::Leaf(*b"INFO", info),
                UcfxNode::Leaf(
                    *b"NAME",
                    names.iter().flat_map(|h| h.to_le_bytes()).collect(),
                ),
                UcfxNode::Leaf(*b"EXMX", exmx),
                UcfxNode::Leaf(*b"PROF", prof),
            ],
        )])
    }

    fn nodes() -> Vec<Node> {
        vec![
            node(0x1, None),
            node(0x10, Some(0)),
            node(0x11, Some(0)),
            node(0x20, Some(0)),
            node(0x21, Some(3)),
            node(0x40, Some(0)),
        ]
    }

    fn parse() -> Mixer {
        let data = mixer_asset();
        let u = Ucfx::parse(&data).unwrap();
        Mixer::parse(&u, 0, &nodes()).unwrap()
    }

    #[test]
    fn reads_parts_profiles_and_pairs() {
        let m = parse();
        assert_eq!(m.version, 1);
        let found: Vec<Option<u16>> = m.parts.iter().map(|p| p.node).collect();
        assert_eq!(found, [Some(1), Some(2), Some(3), None, Some(5)]);
        assert_eq!(m.profiles.len(), 2);
        assert!(!m.together(0, 1) && !m.together(2, 3) && m.together(0, 2));
        assert!(m.is_symmetric());
        assert_eq!(m.allowed(Some(0xB)), [false, true, false, true, true]);
        assert_eq!(m.allowed(Some(0xC)), [true; 5]);
        assert_eq!(m.allowed(None), [true; 5]);
    }

    #[test]
    fn counts_the_looks_a_profile_allows() {
        let m = parse();
        // Two heads times cap or none.
        assert_eq!(m.looks(&m.allowed(Some(0xA)), 100), 4);
        assert_eq!(m.looks(&m.allowed(Some(0xB)), 100), 1);
        assert_eq!(m.looks(&m.allowed(None), 100), 4);
        assert_eq!(m.looks(&m.allowed(None), 3), 3);
    }

    #[test]
    fn picks_one_of_each_group_and_never_two_that_clash() {
        let m = parse();
        let allowed = m.allowed(Some(0xA));
        let mut seen = std::collections::HashSet::new();
        let mut state = 7usize;
        for _ in 0..200 {
            let look = m.pick(&allowed, |k| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state >> 8) % k
            });
            assert_eq!(look.len(), 2);
            assert!(look.iter().any(|&i| i < 2), "a head: {look:?}");
            assert!(look.iter().any(|&i| i == 2 || i == 3), "cap or none");
            assert!(!look.contains(&4), "the profile has no vest");
            seen.insert(look);
        }
        assert_eq!(seen.len(), 4);
        assert_eq!(m.pick(&m.allowed(Some(0xB)), |_| 0), [1, 3, 4]);
    }

    #[test]
    fn hides_the_parts_not_picked_with_what_hangs_under_them() {
        let m = parse();
        // Head 0x11 and the cap: the other head hidden, the cap's child
        // shown with it, the vest (not picked) hidden.
        let hidden = m.hidden(&[1, 2], &nodes());
        assert_eq!(hidden, [false, true, false, false, false, true]);
        let hidden = m.hidden(&[0, 3, 4], &nodes());
        assert_eq!(hidden, [false, false, true, true, true, false]);
    }

    #[test]
    fn a_pair_that_goes_together_one_way_only_clashes() {
        let mut m = parse();
        // The cap goes with the empty pick by its row, not by the empty
        // pick's (as in 11 of the 34 mixers): never both worn.
        let n = m.parts.len();
        m.together[2 * n + 3] = true;
        assert!(!m.is_symmetric());
        assert_eq!(m.looks(&m.allowed(Some(0xA)), 100), 4);
        for k in 0..4 {
            let look = m.pick(&m.allowed(Some(0xA)), |_| k % 2);
            assert!(!(look.contains(&2) && look.contains(&3)), "{look:?}");
        }
    }

    #[test]
    fn rejects_sizes_that_disagree() {
        let mut data = mixer_asset();
        // Part count 6 in INFO: NAME no longer fits.
        let u = Ucfx::parse(&data).unwrap();
        let info = u
            .sections()
            .iter()
            .position(|s| &s.fourcc == b"INFO")
            .unwrap();
        let off = u.sections()[info].data.unwrap().as_ptr() as usize - data.as_ptr() as usize;
        data[off + 2] = 6;
        let u = Ucfx::parse(&data).unwrap();
        assert!(Mixer::parse(&u, 0, &nodes()).is_none());
    }
}
