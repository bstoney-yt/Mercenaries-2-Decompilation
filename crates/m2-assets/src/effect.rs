//! Particle effects: explosions, fire, smoke, dust, sparks and the rest,
//! each a set of emitters with their particles' look and motion.
//!
//! An effect is an asset of type [`EFFECT_TYPE`] (all 314 of them sit in
//! `blocks\VZ\effects_P000_Q3.block`), keyed by [`crate::name_hash`] of its
//! name, in a [`crate::ucfx`] container:
//!
//! ```text
//! EFCT                      the effect
//!   EMTR                    emitter shapes
//!     GEOM                  one shape (see `Shape`)
//!   EMIT                    one emitter, followed by its particles' PTYP
//!     TRFM                  its place: ATRBs
//!     GEOM 4                u16 shape index? (unknown_0), u16 unknown
//!   PTYP                    the particles it gives off
//!     ATRB ...              settings (see `Attr`)
//!     COLR 800              100 colour keys over a particle's life
//!     TEXT                  u32 count, then that many texture name hashes
//!   FRCE                    a force (wind, gravity, drag): ATRBs
//! ```
//!
//! An `ATRB` leaf is 12 bytes: `u32` setting name hash, `u8` kind (1 for a
//! float, 0 for an integer or hash), `u8` flags, `u16` zero, then the
//! 4-byte value. An `ATRB` container instead holds an `ANIM` of `AKEY`s,
//! each `(f32 time, f32 value)`, and carries no name: its setting is known
//! only by its place in the list, which every `PTYP` keeps in one order
//! ([`PARTICLE_ORDER`]), every `TRFM` in another ([`TRANSFORM_ORDER`]) and
//! every `FRCE` starts the same way ([`FORCE_ORDER`]); the parser names a
//! keyed setting by its place.
//!
//! The textures are not texture assets: each hash is a rectangle of the
//! particle atlas, the 2048 x 2048 texture `vfx`, listed by the `fx`
//! dictionary ([`Atlas`], an asset of type [`ATLAS_TYPE`] in the resident
//! block): `INFO` holds the count, `DICT` that many 20-byte entries of
//! `u32` texture hash and `f32` left, bottom, width and height as fractions of
//! the atlas, measured up from its bottom edge (its cells' borders are clear
//! read so: mean alpha 0.005 round the 630 cells, against 0.106 read from
//! the top). An emitter listing several plays them as a flipbook.
//!
//! # Provenance
//!
//! From samples: the layout above, the counts, the place of each setting
//! and the atlas were read from the 314 effects (820 emitters) of a legally
//! installed copy: all 14,509 texture hashes the emitters list are among the
//! dictionary's 630 entries, and none is an asset of its own. Setting names
//! are solved from their hashes ([`setting`]): a name says what a setting
//! is called, while what it does to a particle is inferred from its values
//! and says so where it is used. No executable code was consulted.

use crate::bytes::Reader;
use crate::ucfx::{Ucfx, UcfxError};

/// The asset type hash of particle effects.
pub const EFFECT_TYPE: u32 = 0x5608_bd5a;
/// The asset type hash of the particle atlas dictionary.
pub const ATLAS_TYPE: u32 = 0xfa46_d8a8;
/// The atlas dictionary's name hash (`name_hash("fx")`).
pub const ATLAS_DICT: u32 = 0x86bf_6c5b;
/// The atlas texture's name hash (`name_hash("vfx")`).
pub const ATLAS_TEXTURE: u32 = 0x89e2_11af;

/// Setting name hashes, each `name_hash` of the name it is given here
/// (solved from the hashes; checked by a test). Those called `UNKNOWN_`
/// are not solved; their notes say what their values look like.
pub mod setting {
    pub const NAME: u32 = 0x1de5_c824;
    pub const SIZE: u32 = 0x8c36_54c2;
    pub const SIZE_VAR: u32 = 0xc100_8e25;
    pub const MASS: u32 = 0xc3ae_b321;
    pub const MASS_VAR: u32 = 0x4703_5d90;
    pub const LIFE: u32 = 0xd3ae_67af;
    pub const LIFE_VAR: u32 = 0xc7cf_e6aa;
    pub const FORCE_DRAG: u32 = 0x6cf0_be14;
    pub const FORCE_DRAG_VAR: u32 = 0xbe96_8d3b;
    pub const FORCE_GRAVITY: u32 = 0xc558_c9d8;
    pub const FORCE_GRAVITY_VAR: u32 = 0xd80d_f37f;
    pub const FORCE_WIND: u32 = 0xb3db_b6c0;
    pub const FORCE_WIND_VAR: u32 = 0x4c49_b137;
    pub const SCALE: u32 = 0xeee1_a341;
    pub const RATE: u32 = 0x062f_0d37;
    pub const RATE_VAR: u32 = 0x7065_3182;
    pub const SPREAD: u32 = 0x437f_66ec;
    pub const SPREAD_VAR: u32 = 0xb8a9_5de3;
    pub const SPEED: u32 = 0x15ba_509e;
    pub const SPEED_VAR: u32 = 0x9be6_2e41;
    pub const INHERIT_VEL: u32 = 0xb424_7bf3;
    pub const INHERIT_VEL_VAR: u32 = 0xb15a_bb7e;
    pub const EMISSION_DIR: u32 = 0x270c_9e9d;
    /// Values 0 to 360 (360 in 318 of 820 emitters): a starting turn's
    /// spread in degrees, inferred.
    pub const UNKNOWN_1B878602: u32 = 0x1b87_8602;
    /// Values 0, 90, 180, 540: a turning speed in degrees a second,
    /// inferred.
    pub const UNKNOWN_35ACBAB7: u32 = 0x35ac_bab7;
    pub const POS_X: u32 = 0x7d11_17eb;
    pub const POS_Y: u32 = 0x9b0f_088e;
    pub const POS_Z: u32 = 0xfd16_5e99;
    pub const ROT_X: u32 = 0xf6b5_5f38;
    pub const ROT_Y: u32 = 0x20b7_dfed;
    pub const ROT_Z: u32 = 0x9eb0_5782;
    /// First in every force (values -7.5 to 120): its strength, inferred.
    pub const UNKNOWN_3DC3D9DF: u32 = 0x3dc3_d9df;
    pub const LOCAL: u32 = 0x201b_5a86;
    pub const RANGE: u32 = 0xe068_6a68;
    pub const DECAY: u32 = 0xc278_3a55;
    pub const RADIAL: u32 = 0xb46f_1f0c;
}

/// The order of every `PTYP`'s 32 settings.
pub const PARTICLE_ORDER: [u32; 32] = [
    setting::NAME,
    setting::SIZE,
    setting::SIZE_VAR,
    setting::MASS,
    setting::MASS_VAR,
    setting::LIFE,
    setting::LIFE_VAR,
    0x497a_1895,
    0x1792_b524,
    0x1083_1673,
    0xb619_7efe,
    setting::FORCE_DRAG,
    setting::FORCE_DRAG_VAR,
    setting::FORCE_GRAVITY,
    setting::FORCE_GRAVITY_VAR,
    0x720f_22f8,
    0x4712_719f,
    setting::FORCE_WIND,
    setting::FORCE_WIND_VAR,
    0xc359_2bb7,
    setting::SCALE,
    setting::RATE,
    setting::RATE_VAR,
    setting::SPREAD,
    setting::SPREAD_VAR,
    setting::SPEED,
    setting::SPEED_VAR,
    setting::INHERIT_VEL,
    setting::INHERIT_VEL_VAR,
    setting::UNKNOWN_35ACBAB7,
    setting::UNKNOWN_1B878602,
    setting::EMISSION_DIR,
];

/// The order of every `TRFM`'s 9 settings (the last three, mostly 1, look
/// like a scale).
pub const TRANSFORM_ORDER: [u32; 9] = [
    setting::POS_X,
    setting::POS_Y,
    setting::POS_Z,
    setting::ROT_X,
    setting::ROT_Y,
    setting::ROT_Z,
    0xc557_4c5f,
    0xe355_3d02,
    0x655c_c56d,
];

/// How every `FRCE` starts.
pub const FORCE_ORDER: [u32; 7] = [
    setting::UNKNOWN_3DC3D9DF,
    setting::POS_X,
    setting::POS_Y,
    setting::POS_Z,
    setting::ROT_X,
    setting::ROT_Y,
    setting::ROT_Z,
];

/// One setting.
#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    /// Its name hash; for a keyed (animated) setting, whose container
    /// carries none, the name its place in the list has (`None` past the
    /// known order).
    pub name: Option<u32>,
    /// Byte 4: 1 for a float, 0 for an integer or hash.
    pub kind: u8,
    /// Byte 5.
    pub flags: u8,
    /// The value's bytes, little-endian.
    pub value: [u8; 4],
    /// Keys `(time, value)` of an animated setting.
    pub keys: Vec<(f32, f32)>,
}

impl Attr {
    /// The value as a float.
    pub fn float(&self) -> f32 {
        f32::from_le_bytes(self.value)
    }

    /// The value as an integer.
    pub fn int(&self) -> u32 {
        u32::from_le_bytes(self.value)
    }

    /// Whether it is keyed.
    pub fn keyed(&self) -> bool {
        !self.keys.is_empty()
    }

    /// Its value `time` into whatever its keys count: the keys joined by
    /// straight lines and held past either end; the plain value when it
    /// has none.
    pub fn at(&self, time: f32) -> f32 {
        let Some(&(t0, v0)) = self.keys.first() else {
            return self.float();
        };
        if time <= t0 {
            return v0;
        }
        for w in self.keys.windows(2) {
            let ((ta, va), (tb, vb)) = (w[0], w[1]);
            if time <= tb {
                let span = tb - ta;
                return if span > 0.0 {
                    va + (vb - va) * (time - ta) / span
                } else {
                    vb
                };
            }
        }
        self.keys.last().map_or(v0, |k| k.1)
    }
}

/// The setting called `name` in `list`.
pub fn find(list: &[Attr], name: u32) -> Option<&Attr> {
    list.iter().find(|a| a.name == Some(name))
}

/// One of a particle's 100 colour keys over its life.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourKey {
    /// Stored blue, green, red, alpha (Direct3D's colour order; inferred:
    /// read so, the fires' keys are orange and the smokes' grey).
    pub bgra: [u8; 4],
    /// A half float whose sign and size change with the colour over a
    /// life: fire -20 to -10, glows -12, smoke 1, a dark smoke -1. Read as
    /// the key's brightness, negative for light a particle gives off and
    /// positive for a lit, blended one (inferred; `m2_engine::particles`).
    pub intensity: f32,
    /// The half float after it: 0 in the keys sampled.
    pub unknown_6: f32,
}

impl ColourKey {
    /// Red, green, blue, alpha.
    pub fn rgba(&self) -> [u8; 4] {
        let [b, g, r, a] = self.bgra;
        [r, g, b, a]
    }
}

/// One emitter and the particles it gives off.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Emitter {
    /// `TRFM`: its place.
    pub transform: Vec<Attr>,
    /// The `EMIT`'s `GEOM`, raw: `u16` shape index into
    /// [`Effect::shapes`], `u16` triangle count ([`Emitter::shape`]).
    pub shape_ref: Vec<u8>,
    /// `PTYP`'s settings, in their stored order.
    pub particle: Vec<Attr>,
    /// `COLR`: colour over life (empty if absent).
    pub colour: Vec<ColourKey>,
    /// `TEXT`: texture name hashes, in order.
    pub textures: Vec<u32>,
}

impl Emitter {
    /// The shape its particles start on: an index into
    /// [`Effect::shapes`] and the triangle count stored with it (which
    /// matches the shape's: every one of the 811 emitters with a 4-byte
    /// reference, in the install). `None` for the 9 with none.
    pub fn shape(&self) -> Option<(usize, usize)> {
        let r: [u8; 4] = self.shape_ref.as_slice().try_into().ok()?;
        Some((
            usize::from(u16::from_le_bytes([r[0], r[1]])),
            usize::from(u16::from_le_bytes([r[2], r[3]])),
        ))
    }
}

/// One triangle of an emitter shape: where its particles start.
///
/// Read from the install's 578 shapes (13,148 triangles): a 52-byte
/// entry of 13 floats, the triangle's area (within 1% of the one its
/// corners give), its unit normal (unit on every entry; a third point
/// against the corners' winding, so stored, not derived) and its three
/// corners, in the emitter's own space (metres: a grenade's ring of eight
/// is 0.4 m across, the largest shapes hundreds).
#[derive(Debug, Clone, PartialEq)]
pub struct Triangle {
    pub area: f32,
    pub normal: [f32; 3],
    pub corners: [[f32; 3]; 3],
}

/// Bytes of one [`Triangle`] in an `EMTR` `GEOM` leaf.
const TRIANGLE_LEN: usize = 52;

/// A decoded effect.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effect {
    /// `EMTR`'s `GEOM` leaves, raw: a `u32` triangle count, then that many
    /// [`Triangle`]s ([`Effect::shape_triangles`]).
    pub shapes: Vec<Vec<u8>>,
    pub emitters: Vec<Emitter>,
    /// `FRCE`s.
    pub forces: Vec<Vec<Attr>>,
    /// `TEXT` leaves straight under the effect (a few effects have one).
    pub textures: Vec<u32>,
}

impl Effect {
    /// Shape `index`'s triangles; `None` when there is no such shape or
    /// its bytes don't hold its count of them.
    pub fn shape_triangles(&self, index: usize) -> Option<Vec<Triangle>> {
        let raw = self.shapes.get(index)?;
        let count = usize::try_from(u32::from_le_bytes(raw.get(..4)?.try_into().ok()?)).ok()?;
        let body = raw.get(4..)?;
        if body.len() != count.checked_mul(TRIANGLE_LEN)? {
            return None;
        }
        Some(
            body.chunks_exact(TRIANGLE_LEN)
                .map(|c| {
                    let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
                    let v = |i: usize| [f(i), f(i + 4), f(i + 8)];
                    Triangle {
                        area: f(0),
                        normal: v(4),
                        corners: [v(16), v(28), v(40)],
                    }
                })
                .collect(),
        )
    }
}

/// Errors from decoding an effect.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EffectError {
    #[error(transparent)]
    Container(#[from] UcfxError),
    #[error("no EFCT section")]
    NoEffect,
    #[error("section {0} is malformed")]
    Malformed(&'static str),
}

/// Decodes an effect asset.
pub fn parse(asset: &[u8]) -> Result<Effect, EffectError> {
    let u = Ucfx::parse(asset)?;
    let top = u
        .children(None)
        .find(|&i| &u.sections()[i].fourcc == b"EFCT")
        .ok_or(EffectError::NoEffect)?;
    let mut fx = Effect::default();
    for i in u.children(Some(top)) {
        let s = u.sections()[i];
        match &s.fourcc {
            b"EMTR" => {
                for g in u.children(Some(i)) {
                    if let Some(d) = u.sections()[g].data {
                        fx.shapes.push(d.to_vec());
                    }
                }
            }
            b"EMIT" => {
                let mut e = Emitter::default();
                for c in u.children(Some(i)) {
                    let cs = u.sections()[c];
                    match &cs.fourcc {
                        b"TRFM" => e.transform = attrs(&u, c, &TRANSFORM_ORDER)?,
                        b"GEOM" => e.shape_ref = cs.data.unwrap_or_default().to_vec(),
                        _ => {}
                    }
                }
                fx.emitters.push(e);
            }
            b"PTYP" => {
                // A PTYP belongs to the EMIT before it.
                let e = fx
                    .emitters
                    .last_mut()
                    .ok_or(EffectError::Malformed("PTYP before EMIT"))?;
                e.particle = attrs(&u, i, &PARTICLE_ORDER)?;
                for c in u.children(Some(i)) {
                    let cs = u.sections()[c];
                    match (&cs.fourcc, cs.data) {
                        (b"COLR", Some(d)) => e.colour = colours(d)?,
                        (b"TEXT", Some(d)) => e.textures = hashes(d)?,
                        _ => {}
                    }
                }
            }
            b"FRCE" => fx.forces.push(attrs(&u, i, &FORCE_ORDER)?),
            b"TEXT" => fx.textures = hashes(s.data.unwrap_or_default())?,
            _ => {}
        }
    }
    Ok(fx)
}

/// The `ATRB`s directly under section `parent`.
fn attrs(u: &Ucfx<'_>, parent: usize, order: &[u32]) -> Result<Vec<Attr>, EffectError> {
    let mut out = Vec::new();
    for i in u.children(Some(parent)) {
        let s = u.sections()[i];
        if &s.fourcc != b"ATRB" {
            continue;
        }
        match s.data {
            Some(d) => {
                let mut r = Reader::at(d, 0);
                let name = r.u32().map_err(|_| EffectError::Malformed("ATRB"))?;
                let b = r.take(8).map_err(|_| EffectError::Malformed("ATRB"))?;
                out.push(Attr {
                    name: Some(name),
                    kind: b[0],
                    flags: b[1],
                    value: [b[4], b[5], b[6], b[7]],
                    keys: Vec::new(),
                });
            }
            None => {
                let mut keys = Vec::new();
                for a in u.children(Some(i)) {
                    for k in u.children(Some(a)) {
                        if let Some(d) = u.sections()[k].data {
                            let mut r = Reader::at(d, 0);
                            let t = f32::from_bits(
                                r.u32().map_err(|_| EffectError::Malformed("ATRB"))?,
                            );
                            let v = f32::from_bits(
                                r.u32().map_err(|_| EffectError::Malformed("ATRB"))?,
                            );
                            keys.push((t, v));
                        }
                    }
                }
                out.push(Attr {
                    name: order.get(out.len()).copied(),
                    kind: 1,
                    flags: 0,
                    value: [0; 4],
                    keys,
                });
            }
        }
    }
    Ok(out)
}

fn colours(d: &[u8]) -> Result<Vec<ColourKey>, EffectError> {
    if !d.len().is_multiple_of(8) {
        return Err(EffectError::Malformed("COLR"));
    }
    Ok(d.chunks_exact(8)
        .map(|c| ColourKey {
            bgra: [c[0], c[1], c[2], c[3]],
            intensity: half(u16::from_le_bytes([c[4], c[5]])),
            unknown_6: half(u16::from_le_bytes([c[6], c[7]])),
        })
        .collect())
}

fn hashes(d: &[u8]) -> Result<Vec<u32>, EffectError> {
    let mut r = Reader::at(d, 0);
    let n = r.u32().map_err(|_| EffectError::Malformed("TEXT"))? as usize;
    if d.len() != 4 + n * 4 {
        return Err(EffectError::Malformed("TEXT"));
    }
    (0..n)
        .map(|_| r.u32().map_err(|_| EffectError::Malformed("TEXT")))
        .collect()
}

/// An IEEE half float.
fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let frac = f32::from(h & 0x3ff);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// The particle atlas dictionary: where each texture hash lies on the
/// atlas texture ([`ATLAS_TEXTURE`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Atlas {
    /// Texture hash to `[left, bottom, width, height]` as stored: fractions
    /// of the atlas, measured up from its bottom edge.
    pub rects: std::collections::HashMap<u32, [f32; 4]>,
}

impl Atlas {
    /// Decodes the dictionary asset ([`ATLAS_TYPE`]).
    pub fn parse(asset: &[u8]) -> Result<Self, EffectError> {
        let u = Ucfx::parse(asset)?;
        let bad = || EffectError::Malformed("DICT");
        let info = u.top_level(*b"INFO").ok_or_else(bad)?;
        let dict = u.top_level(*b"DICT").ok_or_else(bad)?;
        let count = Reader::at(info, 0).u32().map_err(|_| bad())? as usize;
        if dict.len() != count.saturating_mul(20) {
            return Err(bad());
        }
        let rects = dict
            .chunks_exact(20)
            .map(|c| {
                let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
                (
                    u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                    [f(4), f(8), f(12), f(16)],
                )
            })
            .collect();
        Ok(Self { rects })
    }

    /// Where texture `hash` lies in texture coordinates, measured down from
    /// the top as images are stored: `[left, top, width, height]`.
    pub fn rect(&self, hash: u32) -> Option<[f32; 4]> {
        let [l, b, w, h] = *self.rects.get(&hash)?;
        Some([l, 1.0 - b - h, w, h])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name_hash;
    use crate::testutil::{UcfxNode, ucfx};

    fn leaf(cc: &[u8; 4], data: Vec<u8>) -> UcfxNode {
        UcfxNode::Leaf(*cc, data)
    }

    fn node(cc: &[u8; 4], children: Vec<UcfxNode>) -> UcfxNode {
        UcfxNode::Container(*cc, children)
    }

    fn atrb(name: u32, value: f32) -> UcfxNode {
        let mut d = name.to_le_bytes().to_vec();
        d.extend([1, 0, 0, 0]);
        d.extend(value.to_le_bytes());
        leaf(b"ATRB", d)
    }

    fn keyed(keys: &[(f32, f32)]) -> UcfxNode {
        let akeys = keys
            .iter()
            .map(|&(t, v)| {
                let mut d = t.to_le_bytes().to_vec();
                d.extend(v.to_le_bytes());
                leaf(b"AKEY", d)
            })
            .collect();
        node(b"ATRB", vec![node(b"ANIM", akeys)])
    }

    #[test]
    fn solved_names_hash_to_their_constants() {
        for (name, hash) in [
            ("Name", setting::NAME),
            ("Size", setting::SIZE),
            ("sizevar", setting::SIZE_VAR),
            ("Mass", setting::MASS),
            ("life", setting::LIFE),
            ("lifevar", setting::LIFE_VAR),
            ("forcegravity", setting::FORCE_GRAVITY),
            ("forcedrag", setting::FORCE_DRAG),
            ("forcewind", setting::FORCE_WIND),
            ("rate", setting::RATE),
            ("ratevar", setting::RATE_VAR),
            ("Spread", setting::SPREAD),
            ("Speed", setting::SPEED),
            ("speedvar", setting::SPEED_VAR),
            ("inheritvel", setting::INHERIT_VEL),
            ("emissiondir", setting::EMISSION_DIR),
            ("PosX", setting::POS_X),
            ("posy", setting::POS_Y),
            ("PosZ", setting::POS_Z),
            ("rotz", setting::ROT_Z),
            ("radial", setting::RADIAL),
            ("fx", ATLAS_DICT),
            ("vfx", ATLAS_TEXTURE),
        ] {
            assert_eq!(name_hash(name), hash, "{name}");
        }
    }

    #[test]
    fn reads_an_emitter_and_names_keyed_settings_by_place() {
        // A PTYP whose size (place 1) is keyed and whose name is plain.
        let mut colr = Vec::new();
        for _ in 0..100 {
            // Stored blue, green, red, alpha, then the half -20.0 and 0.
            colr.extend([0x4d, 0x9e, 0xff, 0xff, 0x00, 0xcd, 0x00, 0x00]);
        }
        let mut text = 2u32.to_le_bytes().to_vec();
        text.extend(0xaaaa_0001u32.to_le_bytes());
        text.extend(0xaaaa_0002u32.to_le_bytes());
        let asset = ucfx(&[node(
            b"EFCT",
            vec![
                node(b"EMTR", vec![leaf(b"GEOM", vec![0; 4])]),
                node(
                    b"EMIT",
                    vec![
                        node(
                            b"TRFM",
                            vec![atrb(setting::POS_X, 0.0), keyed(&[(0.0, 2.5)])],
                        ),
                        leaf(b"GEOM", vec![0, 0, 0x10, 0]),
                    ],
                ),
                node(
                    b"PTYP",
                    vec![
                        atrb(setting::NAME, 0.0),
                        keyed(&[(0.0, 1.0), (50.0, 3.0), (99.0, 3.0)]),
                        leaf(b"COLR", colr),
                        leaf(b"TEXT", text),
                    ],
                ),
                node(b"FRCE", vec![atrb(setting::UNKNOWN_3DC3D9DF, 10.0)]),
            ],
        )]);
        let fx = parse(&asset).unwrap();
        assert_eq!(fx.shapes.len(), 1);
        assert_eq!(fx.forces.len(), 1);
        let e = &fx.emitters[0];
        assert_eq!(e.shape_ref, [0, 0, 0x10, 0]);
        assert_eq!(e.transform[1].name, Some(setting::POS_Y));
        let size = find(&e.particle, setting::SIZE).unwrap();
        assert!(size.keyed());
        assert_eq!(size.at(25.0), 2.0);
        assert_eq!(size.at(-5.0), 1.0);
        assert_eq!(size.at(200.0), 3.0);
        assert_eq!(e.colour.len(), 100);
        assert_eq!(e.colour[0].rgba(), [0xff, 0x9e, 0x4d, 0xff]);
        assert_eq!(e.colour[0].intensity, -20.0);
        assert_eq!(e.textures, [0xaaaa_0001, 0xaaaa_0002]);
    }

    #[test]
    fn a_ptyp_needs_an_emitter() {
        let asset = ucfx(&[node(b"EFCT", vec![node(b"PTYP", vec![atrb(1, 0.0)])])]);
        assert_eq!(
            parse(&asset),
            Err(EffectError::Malformed("PTYP before EMIT"))
        );
    }

    #[test]
    fn reads_the_atlas_dictionary() {
        let mut dict = Vec::new();
        for (h, r) in [
            (7u32, [0.5f32, 0.25, 0.125, 0.0625]),
            (9, [0.0, 0.0, 1.0, 1.0]),
        ] {
            dict.extend(h.to_le_bytes());
            for v in r {
                dict.extend(v.to_le_bytes());
            }
        }
        let asset = ucfx(&[
            leaf(b"INFO", 2u32.to_le_bytes().to_vec()),
            leaf(b"DICT", dict),
        ]);
        let atlas = Atlas::parse(&asset).unwrap();
        // Stored from the bottom; handed back from the top.
        assert_eq!(atlas.rect(7), Some([0.5, 0.6875, 0.125, 0.0625]));
        assert_eq!(atlas.rect(8), None);
        // A count that disagrees with the entries is refused.
        let short = ucfx(&[
            leaf(b"INFO", 3u32.to_le_bytes().to_vec()),
            leaf(b"DICT", vec![0; 40]),
        ]);
        assert!(Atlas::parse(&short).is_err());
    }

    /// A shape of two triangles as the install stores them (count, then
    /// area, normal, corners), and the emitter's reference to it.
    #[test]
    fn shapes_are_triangle_lists_and_emitters_point_at_them() {
        let tri = |area: f32, n: [f32; 3], c: [[f32; 3]; 3]| -> Vec<u8> {
            let mut out = area.to_le_bytes().to_vec();
            for v in n.iter().chain(c.iter().flatten()) {
                out.extend(v.to_le_bytes());
            }
            out
        };
        let mut raw = 2u32.to_le_bytes().to_vec();
        raw.extend(tri(
            0.5,
            [0.0, 1.0, 0.0],
            [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        ));
        raw.extend(tri(
            2.0,
            [0.0, 0.0, -1.0],
            [[1.0, 1.0, 0.0], [3.0, 1.0, 0.0], [1.0, 3.0, 0.0]],
        ));
        let fx = Effect {
            shapes: vec![raw.clone(), vec![1, 0, 0, 0, 0]],
            emitters: vec![
                Emitter {
                    shape_ref: vec![0, 0, 2, 0],
                    ..Emitter::default()
                },
                Emitter::default(),
            ],
            ..Effect::default()
        };
        let t = fx.shape_triangles(0).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].area, t[0].normal), (0.5, [0.0, 1.0, 0.0]));
        assert_eq!(t[1].corners[2], [1.0, 3.0, 0.0]);
        // A shape whose bytes don't hold its count, and a missing one.
        assert_eq!(fx.shape_triangles(1), None);
        assert_eq!(fx.shape_triangles(2), None);
        assert_eq!(fx.emitters[0].shape(), Some((0, 2)));
        assert_eq!(fx.emitters[1].shape(), None);
    }

    #[test]
    fn half_floats() {
        assert_eq!(half(0x3c00), 1.0);
        assert_eq!(half(0xcd00), -20.0);
        assert_eq!(half(0xbc00), -1.0);
        assert_eq!(half(0), 0.0);
    }
}
