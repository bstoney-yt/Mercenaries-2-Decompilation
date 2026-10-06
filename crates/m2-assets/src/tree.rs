//! Trees: the island's palms, canopy and jungle trees, one record per
//! species with every tree of that species placed in it.
//!
//! The tree table (type hash `34612f86`, one asset in the resident block) is
//! a `UCFX` container with a single `tree` leaf:
//!
//! ```text
//! u32 version (0x0b), u32 species count, then the records, packed:
//! 0x00  u32      template id (the species' template, bit 31 set)
//! 0x04  u32 x 2  unknown (4, 2; or 2, 4; or 4, 0)
//! 0x0C  f32      fade distance (m): 200 for most
//! 0x10  u8       unknown (1)
//! 0x11  f32 x 2  scale range (0.7 .. 1.3)
//! 0x19  u32      unknown (0)
//! 0x1D  f32 x 12 three part rects x0, y0, x1, y1 in metres (the palm's
//!                trunk and crown; unused ones FLT_MAX .. -FLT_MAX)
//! 0x4D  f32 x 12 the same parts' UV rects
//! 0x7D  u32      parts in use (1 or 2)
//! 0x81  f32 x 2  crown card width and height (m)
//! 0x89  u32      card vertex count, u32 card index count, then the
//!                vertices (f32 position x 3, f32 UV x 2) and u16 indices:
//!                a cap over the crown (none in two species)
//!       the card's material, if it has vertices, then the parts'
//!       material (three textures): material records as in models
//!       placement block:
//!       u32 stream bytes, the bit stream, u32 stream bits,
//!       f32 x 6 box min, max (world space), u32 unknown (2),
//!       u32 n (trees), a (one-tree leaves), b (two-tree leaves),
//!       c (node bits), g (one-tree leaf bits - 1), g + 1, w (pair split bits),
//!       21 x u32 x 3  x, y, z bits of a position at depth 0 .. 20,
//!       u32 x 3       node split bits per axis,
//!       21 x u32      split axis per depth (0 x, 1 y, 2 z)
//! ```
//!
//! The stream is read most significant bit first within little-endian u32
//! words. It is a complete kd-tree of depth L (a + b = 2^L; n = 2^L when
//! a = b = 0), written depth first, node before children:
//!
//! - a node at depth d splits its box on that depth's axis at a fraction
//!   of the axis's split bits; the low half comes first;
//! - a one-tree leaf is a 4-bit code (2), x, y and z at depth L's bits
//!   within the leaf's box, and a 4-bit tail;
//! - a two-tree leaf is a `w`-bit split on axis L and two one-tree leaves
//!   at depth L + 1's bits, the low half's first.
//!
//! A field of `k` bits holding `q` is `lo + q / (2^k - 1) * (hi - lo)`.
//! The tail is such a field over a full turn: the tree's yaw (see
//! [`TreePlacement::yaw`]). Its values are those of a yaw drawn from 20
//! (in two species 18) even steps of a turn, rounded to fifteenths: every
//! third value is twice as common (every fifth for 18 steps), and 15, a
//! whole turn, is rare except in one species whose yaws are not stepped.
//! As a yaw about +y, the three palms whose crowns lean (`palmtreebend01`,
//! `02`, `03`) lean downhill: their lean's mean resultant against the
//! slope is 0.46, 0.17, 0.14 over 94, 824, 830 trees (0.10, 0.03, 0.03
//! by chance), and within 20 degrees of straight down it. The tail is
//! unrelated to a tree's spacing from its neighbours or its height.
//!
//! Open: how the game tells a one-tree leaf from a
//! two-tree leaf whose split starts with the bits `0010`: in two species
//! some do. [`TreeTable::parse`] resolves those by search: every leaf read
//! as one tree must start with code 2, and the parse must use exactly `a`
//! one-tree and `b` two-tree leaves.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the one tree table in the archives of
//! a legally installed copy. All 30 records parse back to back to the end of
//! the leaf; in every one, the stream's bit count is
//! `c + a (g + 1) + b (2g + w)` and the decode ends on it exactly; the
//! species' counts sum to 36,993 trees, whose heights lie a median 2.7 m
//! from the low-resolution ground (layer-placed objects: 1.3 m). On the
//! full-detail ground within 1.5 km of the Carmona gate, town species
//! stand on it (median 0.00 m), palms on the lowest ground within about
//! 2 m of the trunk, and the jungle species 1-2.6 m below it, spread
//! from 0 to 6 m; the height's quantum is 0.01-0.04 m, so it is not
//! rounding. No bytes from the archives are committed.

use std::collections::HashSet;

use crate::bytes::{OutOfBounds, Reader};
use crate::model::{Material, read_material};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of the tree table.
pub const TREE_TYPE: u32 = 0x3461_2f86;

/// Depths a kd-tree's precision table covers.
const DEPTHS: usize = 21;

/// Errors from parsing the tree table.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TreeError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("missing tree section")]
    Missing,
    #[error("{0} bytes left after the last record")]
    Trailing(usize),
    #[error("record {record}: {what}")]
    Placements { record: usize, what: &'static str },
}

impl From<OutOfBounds> for TreeError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreePlacement {
    /// World space.
    pub position: [f32; 3],
    /// The 4-bit tail, raw: the tree's yaw in fifteenths of a turn
    /// ([`yaw`](Self::yaw)).
    pub tail: u8,
    /// Each coordinate's quantum: its leaf box's side over its field's
    /// steps.
    pub step: [f32; 3],
}

impl TreePlacement {
    /// Yaw about +y, radians: the tail as a 4-bit field over a full turn
    /// (`tail / 15` of it, so 15 is 0 again).
    pub fn yaw(&self) -> f32 {
        f32::from(self.tail) / 15.0 * std::f32::consts::TAU
    }
}

/// One crown card vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

/// One species and its trees.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeSpecies {
    /// Its template (bit 31 set), whose `ModelName` is its model.
    pub template: u32,
    pub unk_0x04: [u32; 2],
    /// Drawn within this many metres.
    pub fade: f32,
    pub unk_0x10: u8,
    /// Scale range.
    pub scale: [f32; 2],
    pub unk_0x19: u32,
    /// Part rects (x0, y0, x1, y1) in metres: trunk, crown, and an empty
    /// third (min > max).
    pub parts: [[f32; 4]; 3],
    /// The parts' UV rects.
    pub part_uvs: [[f32; 4]; 3],
    /// Parts in use (the rects before the empty ones).
    pub part_count: u32,
    /// Crown card width and height (m).
    pub card_size: [f32; 2],
    /// A cap over the crown.
    pub card_vertices: Vec<CardVertex>,
    /// Triangle list.
    pub card_indices: Vec<u16>,
    /// The crown card's material, present with the card.
    pub card_material: Option<Material>,
    /// The parts' material (three textures).
    pub material: Material,
    /// Box of the trees, world space: min, max.
    pub bounds: [[f32; 3]; 2],
    pub trees: Vec<TreePlacement>,
}

/// The tree table.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeTable {
    pub version: u32,
    pub species: Vec<TreeSpecies>,
}

impl TreeTable {
    /// Parses the tree table asset.
    pub fn parse(asset: &[u8]) -> Result<Self, TreeError> {
        let u = Ucfx::parse(asset)?;
        let data = u.top_level(*b"tree").ok_or(TreeError::Missing)?;
        let mut r = Reader::at(data, 0);
        let (version, count) = (r.u32()?, r.u32()?);
        // Each record is well over 64 bytes, which bounds the allocation.
        let mut species = Vec::with_capacity((count as usize).min(data.len() / 64));
        for record in 0..count as usize {
            species.push(read_species(&mut r, record)?);
        }
        if r.remaining() != 0 {
            return Err(TreeError::Trailing(r.remaining()));
        }
        Ok(Self { version, species })
    }
}

fn f32(r: &mut Reader<'_>) -> Result<f32, OutOfBounds> {
    r.u32().map(f32::from_bits)
}

fn rects(r: &mut Reader<'_>) -> Result<[[f32; 4]; 3], OutOfBounds> {
    let mut out = [[0.0; 4]; 3];
    for v in out.iter_mut().flatten() {
        *v = f32(r)?;
    }
    Ok(out)
}

fn read_species(r: &mut Reader<'_>, record: usize) -> Result<TreeSpecies, TreeError> {
    let template = r.u32()?;
    let unk_0x04 = [r.u32()?, r.u32()?];
    let fade = f32(r)?;
    let unk_0x10 = r.u8()?;
    let scale = [f32(r)?, f32(r)?];
    let unk_0x19 = r.u32()?;
    let parts = rects(r)?;
    let part_uvs = rects(r)?;
    let part_count = r.u32()?;
    let card_size = [f32(r)?, f32(r)?];
    let (nv, ni) = (r.u32()? as usize, r.u32()? as usize);
    // Bounded by the bytes left, so a bad count can't allocate much.
    let mut card_vertices = Vec::with_capacity(nv.min(r.remaining() / 20));
    for _ in 0..nv {
        card_vertices.push(CardVertex {
            position: [f32(r)?, f32(r)?, f32(r)?],
            uv: [f32(r)?, f32(r)?],
        });
    }
    let mut card_indices = Vec::with_capacity(ni.min(r.remaining() / 2));
    for _ in 0..ni {
        card_indices.push(r.u16()?);
    }
    let card_material = if nv > 0 {
        Some(read_material(r)?)
    } else {
        None
    };
    let material = read_material(r)?;
    let (bounds, trees) = read_placements(r, record)?;
    Ok(TreeSpecies {
        template,
        unk_0x04,
        fade,
        unk_0x10,
        scale,
        unk_0x19,
        parts,
        part_uvs,
        part_count,
        card_size,
        card_vertices,
        card_indices,
        card_material,
        material,
        bounds,
        trees,
    })
}

/// A placement block's header.
struct Kd<'a> {
    stream: &'a [u8],
    bits: usize,
    /// Single (one-tree) leaves, pair (two-tree) leaves.
    a: usize,
    b: usize,
    /// Bits of a one-tree leaf, of a two-tree leaf.
    single: usize,
    pair: usize,
    w: u32,
    /// Position bits per depth, then (row 21) node split bits per axis.
    prec: [[u32; 3]; DEPTHS + 1],
    axes: [usize; DEPTHS],
    depth: usize,
}

type Placements = ([[f32; 3]; 2], Vec<TreePlacement>);

fn read_placements(r: &mut Reader<'_>, record: usize) -> Result<Placements, TreeError> {
    let bad = |what| TreeError::Placements { record, what };
    let len = r.u32()? as usize;
    let stream = r.take(len)?;
    let bits = r.u32()? as usize;
    let mut b = [0.0; 6];
    for v in &mut b {
        *v = f32(r)?;
    }
    let bounds = [[b[0], b[1], b[2]], [b[3], b[4], b[5]]];
    let _unk = r.u32()?;
    let (n, a, nb, c, g, g1, w) = (
        r.u32()? as usize,
        r.u32()? as usize,
        r.u32()? as usize,
        r.u32()? as usize,
        r.u32()? as usize,
        r.u32()? as usize,
        r.u32()?,
    );
    let mut prec = [[0u32; 3]; DEPTHS + 1];
    for v in prec.iter_mut().flatten() {
        *v = r.u32()?;
    }
    let mut axes = [0usize; DEPTHS];
    for v in &mut axes {
        *v = r.u32()? as usize;
    }
    if bits > stream.len() * 8 || stream.len() % 4 != 0 {
        return Err(bad("stream shorter than its bit count, or not whole words"));
    }
    if axes.iter().any(|&x| x > 2) || prec.iter().flatten().any(|&p| p > 32) || w > 32 {
        return Err(bad("axis or field width out of range"));
    }
    let leaves = if a + nb == 0 { n } else { a + nb };
    if !leaves.is_power_of_two() || (a + nb != 0 && n != a + 2 * nb) {
        return Err(bad("leaf counts are not a complete tree"));
    }
    let depth = leaves.trailing_zeros() as usize;
    if depth >= DEPTHS - 1 {
        return Err(bad("tree too deep"));
    }
    let xyz = |d: usize| prec[d].iter().map(|&p| p as usize).sum::<usize>();
    let single = 8 + xyz(depth);
    let pair = w as usize + 2 * (8 + xyz(depth + 1));
    if a + nb != 0 && (g + 1 != single || g1 != g + 1 || bits != c + a * single + nb * pair) {
        return Err(bad("leaf sizes do not add up to the bit count"));
    }
    let kd = Kd {
        stream,
        bits,
        a,
        b: nb,
        single,
        pair,
        w,
        prec,
        axes,
        depth,
    };
    let kinds = if a + nb == 0 {
        vec![false; leaves]
    } else {
        kd.leaf_kinds()
            .ok_or(bad("no parse uses the leaf counts"))?
    };
    let mut d = Decoder {
        kd: &kd,
        kinds: &kinds,
        leaf: 0,
        pos: 0,
        out: Vec::with_capacity(n),
    };
    d.node(0, bounds[0], bounds[1])
        .ok_or(bad("stream ends early"))?;
    if d.pos != bits || d.out.len() != n {
        return Err(bad("decode does not end on the bit count"));
    }
    Ok((bounds, d.out))
}

impl Kd<'_> {
    /// `n` bits at bit `pos`, most significant first; `None` past the end.
    fn get(&self, pos: usize, n: u32) -> Option<u64> {
        if pos + n as usize > self.bits {
            return None;
        }
        let mut v = 0u64;
        for i in pos..pos + n as usize {
            let w = self.stream.get(i / 32 * 4..i / 32 * 4 + 4)?;
            let w = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
            v = v << 1 | u64::from(w >> (31 - i % 32) & 1);
        }
        Some(v)
    }

    /// Node split bits at depth `d`.
    fn split_bits(&self, d: usize) -> usize {
        self.prec[DEPTHS][self.axes[d]] as usize
    }

    /// Per leaf in stream order, whether it holds two trees: a depth-first
    /// search over (leaf, one-tree leaves so far), whose state fixes where
    /// the leaf starts. A leaf not starting with code 2 holds two trees.
    /// One that does is tried as two trees first if both of its would-be
    /// trees start with code 2 too, else as one.
    fn leaf_kinds(&self) -> Option<Vec<bool>> {
        let leaves = 1usize << self.depth;
        // Node bits before each leaf, and in all.
        let mut nodes = Vec::with_capacity(leaves + 1);
        let mut acc = 0;
        for i in 0..leaves {
            // The nodes opened before leaf i: one per depth below the
            // lowest set bit of i (all of them for leaf 0).
            let opened = if i == 0 {
                self.depth
            } else {
                i.trailing_zeros() as usize
            };
            acc += (self.depth - opened..self.depth)
                .map(|d| self.split_bits(d))
                .sum::<usize>();
            nodes.push(acc);
        }
        nodes.push(acc);
        let start = |i: usize, s: usize| nodes[i] + s * self.single + (i - s) * self.pair;
        let code = |p: usize| self.get(p, 4);
        let w = self.w as usize;
        let options = |p: usize| -> ([bool; 2], usize) {
            match code(p) {
                Some(2) => {
                    let sub = single_tree_len(self);
                    if code(p + w) == Some(2) && code(p + w + sub) == Some(2) {
                        ([true, false], 2)
                    } else {
                        ([false, true], 2)
                    }
                }
                _ => ([true, false], 1),
            }
        };
        struct Frame {
            singles: usize,
            options: [bool; 2],
            count: usize,
            next: usize,
        }
        let mut frames: Vec<Frame> = Vec::with_capacity(leaves);
        let mut dead: HashSet<(usize, usize)> = HashSet::new();
        let mut s = 0;
        loop {
            let i = frames.len();
            let viable = s <= self.a
                && i - s <= self.b
                && !dead.contains(&(i, s))
                && (i == leaves || start(i, s) + 4 <= self.bits);
            if viable && i == leaves {
                if s == self.a && start(i, s) == self.bits {
                    return Some(frames.iter().map(|f| f.options[f.next - 1]).collect());
                }
            } else if viable {
                let (options, count) = options(start(i, s));
                frames.push(Frame {
                    singles: s,
                    options,
                    count,
                    next: 0,
                });
            } else {
                dead.insert((i, s));
            }
            // The next option of the deepest leaf that has one left.
            loop {
                let f = frames.last_mut()?;
                if f.next < f.count {
                    let pair = f.options[f.next];
                    f.next += 1;
                    s = f.singles + usize::from(!pair);
                    break;
                }
                let singles = f.singles;
                frames.pop();
                dead.insert((frames.len(), singles));
            }
        }
    }
}

/// Bits of one tree of a two-tree leaf.
fn single_tree_len(kd: &Kd<'_>) -> usize {
    (kd.pair - kd.w as usize) / 2
}

/// `q` of `bits` bits as a fraction of `lo .. hi`.
fn quantum(q: u64, bits: u32, lo: f32, hi: f32) -> f32 {
    if bits == 0 {
        return lo;
    }
    let top = (1u64 << bits) - 1;
    lo + (q as f64 / top as f64) as f32 * (hi - lo)
}

struct Decoder<'a> {
    kd: &'a Kd<'a>,
    kinds: &'a [bool],
    leaf: usize,
    pos: usize,
    out: Vec<TreePlacement>,
}

impl Decoder<'_> {
    fn read(&mut self, n: u32) -> Option<u64> {
        let v = self.kd.get(self.pos, n)?;
        self.pos += n as usize;
        Some(v)
    }

    /// Splits `lo .. hi` on `axis` at a `bits`-bit fraction.
    fn split(
        &mut self,
        axis: usize,
        bits: u32,
        lo: [f32; 3],
        hi: [f32; 3],
    ) -> Option<([f32; 3], [f32; 3])> {
        let at = quantum(self.read(bits)?, bits, lo[axis], hi[axis]);
        let (mut low_hi, mut high_lo) = (hi, lo);
        low_hi[axis] = at;
        high_lo[axis] = at;
        Some((low_hi, high_lo))
    }

    fn tree(&mut self, prec: [u32; 3], lo: [f32; 3], hi: [f32; 3]) -> Option<()> {
        self.read(4)?;
        let (mut position, mut step) = ([0.0; 3], [0.0; 3]);
        for k in 0..3 {
            position[k] = quantum(self.read(prec[k])?, prec[k], lo[k], hi[k]);
            step[k] = quantum(1, prec[k], 0.0, hi[k] - lo[k]);
        }
        let tail = self.read(4)? as u8;
        self.out.push(TreePlacement {
            position,
            tail,
            step,
        });
        Some(())
    }

    fn node(&mut self, d: usize, lo: [f32; 3], hi: [f32; 3]) -> Option<()> {
        let kd = self.kd;
        if d == kd.depth {
            let pair = *self.kinds.get(self.leaf)?;
            self.leaf += 1;
            if !pair {
                return self.tree(kd.prec[d], lo, hi);
            }
            let (low_hi, high_lo) = self.split(kd.axes[d], kd.w, lo, hi)?;
            self.tree(kd.prec[d + 1], lo, low_hi)?;
            return self.tree(kd.prec[d + 1], high_lo, hi);
        }
        let (low_hi, high_lo) = self.split(kd.axes[d], kd.split_bits(d) as u32, lo, hi)?;
        self.node(d + 1, lo, low_hi)?;
        self.node(d + 1, high_lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    /// Bits written most significant first into little-endian u32 words.
    #[derive(Default)]
    struct BitWriter(Vec<bool>);

    impl BitWriter {
        fn put(&mut self, v: u64, n: u32) {
            for k in (0..n).rev() {
                self.0.push(v >> k & 1 == 1);
            }
        }

        fn words(&self) -> Vec<u8> {
            let mut out = vec![0u8; self.0.len().div_ceil(64) * 8];
            for (i, &b) in self.0.iter().enumerate() {
                if b {
                    let mut w =
                        u32::from_le_bytes(out[i / 32 * 4..i / 32 * 4 + 4].try_into().unwrap());
                    w |= 1 << (31 - i % 32);
                    out[i / 32 * 4..i / 32 * 4 + 4].copy_from_slice(&w.to_le_bytes());
                }
            }
            out
        }
    }

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn floats(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn material(name: u32) -> Vec<u8> {
        let mut m = le(&[name]);
        m.extend(floats(&[1.0; 25]));
        m.extend(0x88u16.to_le_bytes());
        m.extend(1u16.to_le_bytes());
        m.extend(le(&[0x4444, 0x113b_ce29, 0x959d_8470]));
        m
    }

    /// Position bits per depth: 4 at depth 0, one fewer on each split axis.
    const AXES: [u32; DEPTHS] = [0; DEPTHS];

    fn prec(d: usize) -> [u32; 3] {
        [8u32.saturating_sub(d as u32), 4, 8]
    }

    /// A species record around `stream` (a kd-tree `depth` deep), box
    /// 0..256 in x and z, 0..15 in y.
    fn record(template: u32, stream: &BitWriter, depth: usize, counts: [u32; 4]) -> Vec<u8> {
        let mut r = le(&[template, 4, 2]);
        r.extend(floats(&[200.0]));
        r.push(1);
        r.extend(floats(&[0.7, 1.3]));
        r.extend(le(&[0]));
        r.extend(floats(&[0.0; 24]));
        r.extend(le(&[2]));
        r.extend(floats(&[8.0, 20.0]));
        r.extend(le(&[3, 3]));
        r.extend(floats(&[
            0.0, 20.0, 0.0, 0.5, 0.5, 4.0, 18.0, 0.0, 1.0, 0.0, -4.0, 18.0, 0.0, 0.0, 0.0,
        ]));
        r.extend([0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()));
        r.extend(material(0xc39d_ceff));
        r.extend(material(0x42ec_054a));
        let words = stream.words();
        r.extend(le(&[words.len() as u32]));
        r.extend(words);
        r.extend(le(&[stream.0.len() as u32]));
        r.extend(floats(&[0.0, 0.0, 0.0, 256.0, 15.0, 256.0]));
        let [n, a, b, c] = counts;
        let g = 7 + prec(depth).iter().sum::<u32>();
        r.extend(le(&[2, n, a, b, c, g, g + 1, 8]));
        for d in 0..DEPTHS {
            r.extend(le(&prec(d)));
        }
        r.extend(le(&[8, 4, 8]));
        r.extend(le(&AXES));
        r
    }

    fn table(records: &[Vec<u8>]) -> Vec<u8> {
        let mut data = le(&[0x0b, records.len() as u32]);
        for r in records {
            data.extend(r);
        }
        ucfx(&[UcfxNode::Leaf(*b"tree", data)])
    }

    /// One tree at leaf precision `p`: code 2, x, y, z, tail.
    fn tree(s: &mut BitWriter, p: [u32; 3], q: [u64; 3], tail: u64) {
        s.put(2, 4);
        for k in 0..3 {
            s.put(q[k], p[k]);
        }
        s.put(tail, 4);
    }

    /// Depth 1: the root splits x at 128/255 of the box, a one-tree leaf
    /// on the low side and a two-tree leaf on the high side.
    fn mixed() -> BitWriter {
        let mut s = BitWriter::default();
        s.put(0x80, 8); // root split
        tree(&mut s, prec(1), [0, 15, 127], 5);
        s.put(0xff, 8); // pair split at the leaf's high x edge
        tree(&mut s, prec(2), [63, 0, 0], 14);
        tree(&mut s, prec(2), [0, 0, 255], 0);
        s
    }

    #[test]
    fn parses_species_and_trees() {
        let a = table(&[record(0x8000_4dac, &mixed(), 1, [3, 1, 1, 8])]);
        let t = TreeTable::parse(&a).unwrap();
        assert_eq!(t.version, 0x0b);
        let sp = &t.species[0];
        assert_eq!(
            (sp.template, sp.fade, sp.scale),
            (0x8000_4dac, 200.0, [0.7, 1.3])
        );
        assert_eq!(sp.card_vertices.len(), 3);
        assert_eq!(sp.card_vertices[1].position, [4.0, 18.0, 0.0]);
        assert_eq!(sp.card_indices, [0, 1, 2]);
        assert_eq!(sp.material.name_hash, 0x42ec_054a);
        assert_eq!(sp.card_material.as_ref().unwrap().name_hash, 0xc39d_ceff);
        assert_eq!(sp.bounds[1], [256.0, 15.0, 256.0]);
        let tr = &sp.trees;
        assert_eq!(tr.len(), 3);
        assert_eq!(tr[0].tail, 5);
        assert_eq!(tr[0].position, [0.0, 15.0, 127.0 / 255.0 * 256.0]);
        // The pair splits at the leaf's high x: tree A spans the leaf.
        assert!((tr[1].position[0] - 256.0).abs() < 1e-3);
        assert_eq!((tr[1].tail, tr[2].tail), (14, 0));
        assert_eq!(tr[2].position[2], 256.0);
        // x at depth 1 has 7 bits over the low half's 128/255 * 256.
        assert!((tr[0].step[0] - 128.0 / 255.0 * 256.0 / 127.0).abs() < 1e-4);
        assert_eq!(tr[2].yaw(), 0.0);
        assert!((tr[0].yaw() - std::f32::consts::TAU / 3.0).abs() < 1e-6);
    }

    #[test]
    fn only_one_tree_leaves() {
        let mut s = BitWriter::default();
        s.put(0x40, 8);
        tree(&mut s, prec(1), [127, 0, 0], 1);
        tree(&mut s, prec(1), [0, 0, 0], 2);
        let a = table(&[record(1, &s, 1, [2, 0, 0, 0])]);
        let t = TreeTable::parse(&a).unwrap();
        let tr = &t.species[0].trees;
        assert_eq!(tr[0].position[0], 64.0 / 255.0 * 256.0);
        assert_eq!(tr[1].position[0], 64.0 / 255.0 * 256.0);
    }

    #[test]
    fn a_one_tree_leaf_that_reads_as_a_pair_is_resolved_by_the_counts() {
        // Depth 2: one-tree leaves S0 and S1, then two-tree leaves D2, D3.
        // S0's bits at 8 and S1's at 7 (S0's 33) read 0010, so S0 also
        // looks like a pair whose two trees start with code 2. Read so, the
        // next leaf starts on 0011, then 0000: three pairs, one too many.
        let mut s = BitWriter::default();
        s.put(0x80, 8);
        s.put(0x80, 8);
        tree(&mut s, prec(2), [0b000100, 0b1000, 0], 1);
        tree(&mut s, prec(2), [0b000001, 0b0011, 0], 2);
        s.put(0x80, 8);
        s.put(0xc0, 8);
        tree(&mut s, prec(3), [0, 0, 0], 3);
        tree(&mut s, prec(3), [0, 0, 0], 4);
        s.put(0x80, 8);
        tree(&mut s, prec(3), [0, 0, 0], 5);
        tree(&mut s, prec(3), [0, 0, 0], 6);
        let a = table(&[record(1, &s, 2, [6, 2, 2, 24])]);
        let t = TreeTable::parse(&a).unwrap();
        let tails: Vec<u8> = t.species[0].trees.iter().map(|t| t.tail).collect();
        assert_eq!(tails, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn inconsistent_counts_are_errors() {
        let s = mixed();
        let a = table(&[record(1, &s, 1, [3, 1, 1, 9])]);
        assert!(matches!(
            TreeTable::parse(&a),
            Err(TreeError::Placements { record: 0, .. })
        ));
        let a = table(&[record(1, &s, 1, [3, 2, 1, 8])]);
        assert!(TreeTable::parse(&a).is_err());
    }

    #[test]
    fn byte_flips_never_panic() {
        let a = table(&[record(1, &mixed(), 1, [3, 1, 1, 8])]);
        for at in 0..a.len() {
            for v in [0, 1, 2, 0x7f, 0xff] {
                let mut b = a.clone();
                b[at] = v;
                let _ = TreeTable::parse(&b);
            }
        }
    }
}
