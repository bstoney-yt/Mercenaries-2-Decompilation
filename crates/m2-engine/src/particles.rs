//! The game's own particle effects, run on the CPU: which effect each
//! template shows, and the particles its emitters give off, as sprites to
//! draw.
//!
//! [`Library`] holds every effect ([`m2_assets::effect`]), the particle
//! atlas, and each template's effects (its `RedEffectComponent` records'
//! `Name`, which is an effect's name hash: `Explosion (Grenade)` names the
//! grenade's fireball). [`Particles`] runs effects placed in the world and
//! hands back [`Sprite`]s: camera-facing squares cut from the atlas.
//!
//! # Provenance
//!
//! From samples: the effect format, the atlas, the template link (536
//! `RedEffectComponent` records name 312 of the 314 effects), colour keys
//! read blue-green-red-alpha, a flipbook of an emitter's textures, and keyed
//! settings' clocks (the size's keys run 0 to 99, a particle's life in
//! hundredths; the others' run up to 510, the effect's time in frames).
//!
//! Inferred from values and the reference footage, not checked against the
//! game's code:
//! - blending: a colour key's [`intensity`](m2_assets::effect::ColourKey)
//!   below zero is light the particle gives off (added to the scene,
//!   `|intensity|` times its colour: fire, glows, sparks); above zero a
//!   lit, alpha-blended particle (smoke, dust, dirt). The footage's
//!   fireballs blow out to yellow-white, as an added colour ten to twenty
//!   times over would. The atlas agrees: where their cells are clear
//!   (alpha under 0.1) the 182 emitters keyed below zero are painted black
//!   (median brightness 0.01), as added textures are, while the 638 keyed
//!   above zero carry colour there (median 0.37) that only blending hides;
//! - `Speed` in metres a second,
//!   `Spread` a cone's half angle in degrees about the emitter's up,
//!   `life` in seconds, each `var` a plus-or-minus spread.
//!
//! From the 360 code (its sprite fill, 0x821E3F38) and the particle vertex
//! shaders (the PC's 346; the 360's PgFXVP, whose only corner constants
//! are plus and minus 0.5): a sprite's width is its `Size` at its age
//! (keys over 0 to 99) plus its `sizevar` times its own draw, in metres;
//! its colour runs between neighbouring keys, its brightness factor that
//! of the key below. (The alpha had taken `Size` as half the width,
//! fitted to footage of small fires; drawn as the shaders have it, the
//! satellite bomb's fireball is about half the size of Brendan's footage
//! of one, which stays unexplained.)
//!
//! From the 360 code: how particles move once given off, by the effect's
//! forces and their emitter's mass and per-kind scales ([`ForceSpec`],
//! [`LiveForce`], [`push`]).
//!
//! Invented (no data decides them yet): how long a constant-rate emitter runs in a one-off effect
//! ([`ONE_OFF`]), which way a particle leaves its shape (along its
//! patch's normal; inferred) and the particle budgets.

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::effect::{
    self, ATLAS_DICT, ATLAS_TYPE, Atlas, Attr, ColourKey, EFFECT_TYPE, Effect, ForceHead,
    ForceKind, setting,
};
use m2_assets::{Layer, name_hash};
use m2_math::glam::{EulerRot, Quat, Vec3};

use crate::assets::{AssetError, AssetSource};
use crate::tuning::Table;

/// The clock an effect's keyed settings count in: frames a second. From
/// the 360 code: its curve reader (0x821DF920) takes the effect's time in
/// seconds times the constant 60.0 at 0x82B7D884 (read, never written).
pub const FRAME: f32 = 60.0;
/// How long a constant-rate emitter gives off particles when its effect is
/// set off once (an explosion's), seconds. Invented.
pub const ONE_OFF: f32 = 0.5;
/// How long a held effect runs past its source's last call, seconds
/// (sources call every 0.3 to 0.4 s). Invented.
pub const HOLD: f32 = 0.6;
/// Most particles one emitter gives off in one step, and most in the world.
/// From the 360 code (the spawn counts, 0x821DBE70 and 0x821DE7A0).
const EMITTER_BUDGET: usize = 2_000;
const BUDGET: usize = 15_000;

/// One patch of an emitter's shape ([`m2_assets::effect::Triangle`]),
/// where its particles start: its corners and normal in the emitter's
/// own space, and the shape's area up to and including it, for picking
/// a patch by area.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub corners: [Vec3; 3],
    pub normal: Vec3,
    pub up_to: f32,
}

/// A per-particle setting and its spread: each particle takes `value +
/// var x u`, `u` its own draw from -1 to 1 (from the 360 code: one random
/// byte a particle, `u = (0.5 - byte / 255) x 2`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spread {
    pub value: f32,
    pub var: f32,
}

impl Spread {
    /// The setting for a particle whose draw is `u`.
    pub fn at(self, u: f32) -> f32 {
        self.value + self.var * u
    }
}

/// One emitter's settings, read once.
#[derive(Debug, Clone)]
pub struct EmitterSpec {
    /// Where it sits in the effect, metres.
    pub offset: Vec3,
    /// Its turn (`TRFM`'s rotation), which its shape and axis follow.
    pub pose: Quat,
    /// The way it points (its turn applied to up).
    pub axis: Vec3,
    /// The shape its particles start on, as the effect's `EMTR` stores
    /// it (from samples); empty for the 9 emitters without one, which
    /// start at the emitter. That a particle flies out along its patch's
    /// normal is inferred: the normals are stored per patch, not derived.
    pub shape: Vec<Patch>,
    pub size: Attr,
    pub size_var: f32,
    pub life: Attr,
    pub life_var: f32,
    pub rate: Attr,
    pub rate_var: f32,
    pub speed: Attr,
    pub speed_var: f32,
    pub spread: Attr,
    pub spread_var: f32,
    /// A particle's mass and how much each kind of force moves it
    /// (`forcegravity`, `forcedrag`, `forcewind`, and the two unnamed
    /// attractor and vortex scales), each with its spread; none keyed in
    /// the PC data. Read where the 360 step reads them (from the 360 code,
    /// 0x827DD6A0: emitter +0x160 mass, +0x174 attractor, +0x188 vortex,
    /// +0x19c drag, +0x1b0 wind, +0x1c4 gravity, each spread 12 bytes on).
    pub mass: Spread,
    pub gravity: Spread,
    pub drag: Spread,
    pub wind: Spread,
    pub attract: Spread,
    pub swirl: Spread,
    /// Starting turn's spread and turning speed, degrees (inferred).
    pub turn: f32,
    pub spin: f32,
    /// Atlas rectangles, in flipbook order (textures not in the atlas
    /// left out).
    pub frames: Vec<[f32; 4]>,
    /// Texture hashes it lists, and how many of them the atlas has.
    pub textures: usize,
    pub found: usize,
    pub colour: Vec<ColourKey>,
}

impl EmitterSpec {
    fn read(e: &effect::Emitter, fx: &Effect, atlas: &Atlas) -> Self {
        let p = &e.particle;
        let mut up_to = 0.0;
        let shape: Vec<Patch> = e
            .shape()
            .and_then(|(index, _)| fx.shape_triangles(index))
            .unwrap_or_default()
            .iter()
            .map(|t| {
                up_to += t.area.max(0.0);
                Patch {
                    corners: t.corners.map(Vec3::from_array),
                    normal: Vec3::from_array(t.normal),
                    up_to,
                }
            })
            .collect();
        let attr = |name| effect::find(p, name).cloned().unwrap_or_else(|| zero(name));
        let num = |name| effect::find(p, name).map_or(0.0, |a| a.at(0.0));
        let spread = |name, var| Spread {
            value: num(name),
            var: num(var),
        };
        let t = &e.transform;
        let place = |name| effect::find(t, name).map_or(0.0, |a| a.at(0.0));
        let turn = Quat::from_euler(
            EulerRot::YXZ,
            place(setting::ROT_Y).to_radians(),
            place(setting::ROT_X).to_radians(),
            place(setting::ROT_Z).to_radians(),
        );
        let frames: Vec<[f32; 4]> = e.textures.iter().filter_map(|&h| atlas.rect(h)).collect();
        Self {
            offset: Vec3::new(
                place(setting::POS_X),
                place(setting::POS_Y),
                place(setting::POS_Z),
            ),
            pose: turn,
            axis: turn * Vec3::Y,
            shape,
            size: attr(setting::SIZE),
            size_var: num(setting::SIZE_VAR),
            life: attr(setting::LIFE),
            life_var: num(setting::LIFE_VAR),
            rate: attr(setting::RATE),
            rate_var: num(setting::RATE_VAR),
            speed: attr(setting::SPEED),
            speed_var: num(setting::SPEED_VAR),
            spread: attr(setting::SPREAD),
            spread_var: num(setting::SPREAD_VAR),
            mass: spread(setting::MASS, setting::MASS_VAR),
            gravity: spread(setting::FORCE_GRAVITY, setting::FORCE_GRAVITY_VAR),
            drag: spread(setting::FORCE_DRAG, setting::FORCE_DRAG_VAR),
            wind: spread(setting::FORCE_WIND, setting::FORCE_WIND_VAR),
            attract: spread(setting::UNKNOWN_10831673, setting::UNKNOWN_B6197EFE),
            swirl: spread(setting::UNKNOWN_720F22F8, setting::UNKNOWN_4712719F),
            turn: num(setting::UNKNOWN_1B878602),
            spin: num(setting::UNKNOWN_35ACBAB7),
            textures: e.textures.len(),
            found: frames.len(),
            frames,
            colour: e.colour.clone(),
        }
    }

    /// Whether its particles give off light (added to the scene) rather
    /// than being blended over it: its keys' brightness is mostly below
    /// zero (inferred).
    pub fn glows(&self) -> bool {
        let below = self.colour.iter().filter(|k| k.intensity < 0.0).count();
        below * 2 > self.colour.len()
    }

    /// Whether its emission rate is keyed to the effect's time (an
    /// explosion's burst) rather than steady (a fire's).
    pub fn bursts(&self) -> bool {
        self.rate.keyed()
    }

    /// The effect's time, seconds, after which a burst gives off no more.
    fn burst_end(&self) -> f32 {
        self.rate.keys.last().map_or(0.0, |k| k.0 / FRAME)
    }
}

fn zero(name: u32) -> Attr {
    Attr {
        name: Some(name),
        kind: 1,
        flags: 0,
        value: [0; 4],
        keys: Vec::new(),
    }
}

/// An effect ready to run.
#[derive(Debug, Clone)]
pub struct EffectSpec {
    pub hash: u32,
    pub emitters: Vec<EmitterSpec>,
    /// Its forces ([`ForceSpec`]); none on 24 of the 314 effects, whose
    /// particles keep their velocity.
    pub forces: Vec<ForceSpec>,
}

impl EffectSpec {
    pub fn read(hash: u32, fx: &Effect, atlas: &Atlas) -> Self {
        Self {
            hash,
            emitters: fx
                .emitters
                .iter()
                .map(|e| EmitterSpec::read(e, fx, atlas))
                .collect(),
            forces: fx
                .forces
                .iter()
                .enumerate()
                .map(|(i, f)| ForceSpec::read(f, fx.force_heads.get(i)))
                .collect(),
        }
    }
}

/// One of an effect's forces, its settings keyed over the effect's time.
///
/// From the 360 code (the per-frame setup, 0x821D9110): each frame every
/// setting is evaluated at the effect's time; the force's place is its
/// position and turn (degrees) on the effect's; a gravity pulls along its
/// turn applied to (0, -1, 0) (the constant at 0x82de896c), every other
/// kind points along its turn applied to up; a drag's strength is
/// strength x pi x radius squared; a wind ignores its own settings and
/// takes the world's wind (the alpha has none, so a wind does nothing).
/// The particle step then pulls a gravity straight down the world all the
/// same ([`LiveForce::add`]).
#[derive(Debug, Clone)]
pub struct ForceSpec {
    /// Its kind: from its tag ([`ForceHead`]; every force in the data has
    /// one), else guessed from its settings ([`ForceKind::infer`]).
    pub kind: ForceKind,
    /// Whether [`kind`](Self::kind) came from its tag.
    pub tagged: bool,
    pub strength: Attr,
    pub radius: Attr,
    pub range: Attr,
    pub decay: Attr,
    pub radial: Attr,
    pub local: Attr,
    pub position: [Attr; 3],
    pub turn: [Attr; 3],
}

/// A force as one frame has it ([`ForceSpec::live`]): kind, place, the
/// way it points, and its evaluated settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveForce {
    pub kind: ForceKind,
    pub at: Vec3,
    pub direction: Vec3,
    /// Its turn, the frame a local vortex works in.
    pub turn: Quat,
    pub strength: f32,
    pub range: f32,
    pub decay: f32,
    pub radial: f32,
    /// Whether it fades out at its range (its `local` setting; the 360
    /// step reads it so for attractors and vortices).
    pub local: bool,
}

impl ForceSpec {
    fn read(attrs: &[Attr], head: Option<&ForceHead>) -> Self {
        let tag = head.and_then(ForceHead::kind);
        let get = |name| {
            effect::find(attrs, name)
                .cloned()
                .unwrap_or_else(|| zero(name))
        };
        Self {
            kind: tag.unwrap_or_else(|| ForceKind::infer(attrs)),
            tagged: tag.is_some(),
            strength: get(setting::UNKNOWN_3DC3D9DF),
            radius: get(setting::DRAG_RADIUS),
            range: get(setting::RANGE),
            decay: get(setting::DECAY),
            radial: get(setting::RADIAL),
            local: get(setting::LOCAL),
            position: [setting::POS_X, setting::POS_Y, setting::POS_Z].map(get),
            turn: [setting::ROT_X, setting::ROT_Y, setting::ROT_Z].map(get),
        }
    }

    /// The force at `frame` (the effect's time in keyed frames), for an
    /// effect at `origin` drawn `scale` times its size.
    pub fn live(&self, frame: f32, origin: Vec3, scale: f32) -> LiveForce {
        let v = |a: &Attr| a.at(frame);
        let turn = Quat::from_euler(
            EulerRot::YXZ,
            v(&self.turn[1]).to_radians(),
            v(&self.turn[0]).to_radians(),
            v(&self.turn[2]).to_radians(),
        );
        let down = matches!(self.kind, ForceKind::Gravity);
        let direction = turn * if down { Vec3::NEG_Y } else { Vec3::Y };
        let mut strength = v(&self.strength);
        if self.kind == ForceKind::Drag {
            let r = v(&self.radius);
            strength *= std::f32::consts::PI * r * r;
        }
        if self.kind == ForceKind::Wind {
            strength = 0.0;
        }
        LiveForce {
            kind: self.kind,
            at: origin
                + Vec3::new(
                    v(&self.position[0]),
                    v(&self.position[1]),
                    v(&self.position[2]),
                ) * scale,
            direction,
            turn,
            strength,
            range: v(&self.range) * scale,
            decay: v(&self.decay),
            radial: v(&self.radial),
            local: v(&self.local) != 0.0,
        }
    }
}

impl LiveForce {
    /// What the force adds to the push on a particle of emitter `e` at
    /// `at` moving at `velocity`, its draw `u` and mass `mass`, and to the
    /// drag and wind sums the step keeps ([`push`]).
    ///
    /// From the 360 code (the particle step, 0x827DD6A0, VMX128 vector
    /// code read by hand), `r` the particle's place less the force's, `d`
    /// its length:
    /// - gravity: `(0, -1, 0) x forcegravity x strength x mass`;
    /// - wind: along the force's direction by `strength x forcewind`; it
    ///   also adds the square root of its strength to the wind sum;
    /// - drag: adds `forcedrag x strength` to the drag sum;
    /// - attractor: `r x strength x scale x fade / (d^3 + 1)` for a local
    ///   force, `r x strength x scale / (d^2 + 1)` otherwise; `fade` is
    ///   nothing past the range `R`, else `(1 - d/R)^decay` for a decay
    ///   above 0 and `(d/R)^-decay` otherwise (0 less the decay: the 360
    ///   code subtracts from its zero register, 0x827DE134);
    /// - vortex, not local: `(radial x r / d + r x v) x strength x scale /
    ///   (d^3 + 1)`; local, in the force's own frame: `5 x radial` along
    ///   the particle's way out from the frame's x axis, plus `w x v`,
    ///   turned back to the world and times its scale. `w` is meant to be
    ///   `(strength x fade, 0, 0)`, but the 360 code picks it with the
    ///   bits of the float 2^32 (0x4f800000, at 0x82B4D8A0) where a mask
    ///   of ones was surely meant; the same bits pick the place's x out of
    ///   the way out. Kept as the 360 code has it ([`masked`]).
    fn add(&self, e: &EmitterSpec, at: Vec3, velocity: Vec3, u: f32, mass: f32, sums: &mut Sums) {
        let fade = |d: f32| {
            if d > self.range {
                0.0
            } else if self.decay > 0.0 {
                (1.0 - d / self.range).powf(self.decay)
            } else {
                (d / self.range).powf(-self.decay)
            }
        };
        match self.kind {
            ForceKind::Gravity => {
                sums.push += Vec3::NEG_Y * e.gravity.at(u) * self.strength * mass;
            }
            ForceKind::Wind => {
                sums.push += self.direction * self.strength * e.wind.at(u);
                sums.wind += self.strength.sqrt();
            }
            ForceKind::Drag => sums.drag += e.drag.at(u) * self.strength,
            ForceKind::Attractor => {
                let r = at - self.at;
                let d2 = r.length_squared();
                let d = d2.sqrt();
                let s = self.strength * e.attract.at(u);
                sums.push += if self.local {
                    r * (s * fade(d)) / (d * d2 + 1.0)
                } else {
                    r * s / (d2 + 1.0)
                };
            }
            ForceKind::Vortex => {
                let r = at - self.at;
                let d2 = r.length_squared();
                let d = d2.sqrt();
                let scale = e.swirl.at(u);
                if self.local {
                    let back = self.turn.inverse();
                    let here = back * r;
                    let moving = back * velocity;
                    let out = Vec3::new(masked(here.x, false), here.y, here.z);
                    let out = out / out.length();
                    let w = Vec3::new(masked(self.strength * fade(d), true), 0.0, 0.0);
                    let local = out * (self.radial * 5.0) + w.cross(moving);
                    sums.push += self.turn * local * scale;
                } else {
                    let turn = r / d * self.radial + r.cross(velocity);
                    sums.push += turn * (self.strength * scale) / (d * d2 + 1.0);
                }
            }
        }
    }
}

/// The 360 vortex's select by the bits of 0x4f800000 ([`LiveForce::add`]):
/// with `keep`, only those bits of `x` (for most values a power of two
/// near it, tiny below 2); without, every other bit (for most values a
/// number near 0).
fn masked(x: f32, keep: bool) -> f32 {
    const BITS: u32 = 0x4f80_0000;
    f32::from_bits(if keep {
        x.to_bits() & BITS
    } else {
        x.to_bits() & !BITS
    })
}

/// What a particle's forces add up to in one step ([`LiveForce::add`]).
#[derive(Debug, Clone, Copy, Default)]
struct Sums {
    push: Vec3,
    drag: f32,
    wind: f32,
}

/// One step of `dt` seconds for a particle at `at` moving at `velocity`,
/// its emitter `e` and draw `u`, under `forces`: its new velocity and
/// place. From the 360 code (0x827DD6A0): with the forces' push `F`, mass
/// `m`, speed `s` and drag `k` (the drag sum, times the wind sum when
/// there is wind), cut to `m / (s dt)` where `k s dt` would pass `m`, the
/// new velocity is `v + (F - k s v) (dt / m) / (1 + k dt / m)` and the
/// new place `at + v' dt`. A particle without forces keeps its velocity.
pub fn push(
    forces: &[LiveForce],
    e: &EmitterSpec,
    at: Vec3,
    velocity: Vec3,
    u: f32,
    dt: f32,
) -> (Vec3, Vec3) {
    let mass = e.mass.at(u);
    let mut sums = Sums::default();
    for f in forces {
        f.add(e, at, velocity, u, mass, &mut sums);
    }
    let k = if sums.wind > 0.0 {
        sums.drag * sums.wind
    } else {
        sums.drag
    };
    let speed = velocity.length();
    let k = if k * speed * dt > mass {
        mass / (speed * dt)
    } else {
        k
    };
    let step = dt / mass;
    let v = velocity + (sums.push - velocity * (k * speed)) * step / (1.0 + k * step);
    (v, at + v * dt)
}

/// Every effect, the atlas, and which effects each template shows.
#[derive(Debug, Clone, Default)]
pub struct Library {
    pub effects: HashMap<u32, EffectSpec>,
    pub atlas: Atlas,
    /// Template id to the effects its `RedEffectComponent`s name.
    pub by_template: HashMap<u32, Vec<u32>>,
    /// How long each of those runs, in the same order ([`template_runs`]);
    /// empty: each runs [`ONE_OFF`].
    pub runs: HashMap<u32, Vec<EffectRun>>,
}

impl Library {
    /// Reads every effect and the atlas dictionary; `by_template` is the
    /// template database's ([`template_effects`]).
    pub fn load<R: Read + Seek>(
        source: &mut AssetSource<R>,
        by_template: HashMap<u32, Vec<u32>>,
    ) -> Result<Self, AssetError> {
        let atlas_data = source.asset(ATLAS_DICT, ATLAS_TYPE)?;
        let atlas = Atlas::parse(&atlas_data).map_err(|e| AssetError::Effect {
            hash: ATLAS_DICT,
            source: e,
        })?;
        let index = source.archive().index();
        let mut hashes: Vec<u32> = index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == EFFECT_TYPE)
            .map(|a| a.name_hash)
            .collect();
        hashes.sort_unstable();
        hashes.dedup();
        let mut effects = HashMap::new();
        for h in hashes {
            let data = source.asset(h, EFFECT_TYPE)?;
            match effect::parse(&data) {
                Ok(fx) => {
                    effects.insert(h, EffectSpec::read(h, &fx, &atlas));
                }
                Err(e) => eprintln!("effect {h:08x}: {e}"),
            }
        }
        Ok(Self {
            effects,
            atlas,
            by_template,
            runs: HashMap::new(),
        })
    }

    /// The effects template `id` shows.
    pub fn of_template(&self, id: u32) -> &[u32] {
        self.by_template.get(&id).map_or(&[], Vec::as_slice)
    }

    /// How long the `k`th effect template `id` shows runs, if known.
    pub fn run(&self, id: u32, k: usize) -> Option<EffectRun> {
        self.runs.get(&id)?.get(k).copied()
    }
}

/// Each template's effects: the `Name` of its `RedEffectComponent`
/// records, by template id.
pub fn template_effects(db: &Layer<'_>) -> HashMap<u32, Vec<u32>> {
    let t = Table::new(db, "RedEffectComponent");
    let name = name_hash("Name");
    t.templates()
        .map(|id| {
            let fx: Vec<u32> = t
                .of_template(id)
                .iter()
                .filter_map(|r| t.word(r, name))
                .filter(|&h| h != 0)
                .collect();
            (id, fx)
        })
        .filter(|(_, fx)| !fx.is_empty())
        .collect()
}

/// How long one of a template's effects runs (its `RedEffectComponent`
/// record's `Duration`, `DurationVar` and `AutoDelete`). From the 360
/// code: an effect gives off particles while it is no older than its
/// `Duration` (0x821DA9D8, the component's tick), and one that deletes
/// itself lives `Duration` give or take up to `DurationVar`, evenly
/// (0x821D7B10: `Duration + (0.5 - rand) * 2 * DurationVar`), and goes
/// with the entity once that is up; one that doesn't stays, given off
/// no more. `global_particle_firelarge` 25 s +-10, `fx_CollapseBits`
/// 3 s, a wreck's `global_particle_blastdoors_ext_smoke` 100 s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectRun {
    pub duration: f32,
    pub var: f32,
    pub auto_delete: bool,
}

/// Each template's effects' runs, in [`template_effects`]' order.
pub fn template_runs(db: &Layer<'_>) -> HashMap<u32, Vec<EffectRun>> {
    let t = Table::new(db, "RedEffectComponent");
    let (name, duration, var, auto) = (
        name_hash("Name"),
        name_hash("Duration"),
        name_hash("DurationVar"),
        name_hash("AutoDelete"),
    );
    t.templates()
        .map(|id| {
            let runs: Vec<EffectRun> = t
                .of_template(id)
                .iter()
                .filter(|r| t.word(r, name).is_some_and(|h| h != 0))
                .map(|r| EffectRun {
                    duration: t.float(r, duration).unwrap_or(0.0),
                    var: t.float(r, var).unwrap_or(0.0),
                    auto_delete: t.word(r, auto).is_some_and(|w| w != 0),
                })
                .collect();
            (id, runs)
        })
        .filter(|(_, runs)| !runs.is_empty())
        .collect()
}

/// One particle to draw: a square facing the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    pub at: Vec3,
    /// Its width, metres.
    pub size: f32,
    /// Turn about the view direction, radians.
    pub turn: f32,
    /// Atlas rectangle: left, top, width, height.
    pub uv: [f32; 4],
    /// Colour, brightness included, and alpha: linear light for a blended
    /// particle, display values for a glow (the renderer adds glows in
    /// display values, as the game does).
    pub colour: [f32; 4],
    /// Added to the scene rather than blended over it.
    pub glow: bool,
}

#[derive(Debug, Clone, Copy)]
struct Particle {
    at: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    /// Its effect's scale.
    scale: f32,
    turn: f32,
    spin: f32,
    emitter: u16,
    /// Its own draw, -1 to 1, which its spreads take ([`Spread`]).
    draw: f32,
}

#[derive(Debug, Clone)]
struct Instance {
    effect: u32,
    /// The source keeping it going, for a held one.
    key: Option<u64>,
    at: Vec3,
    scale: f32,
    age: f32,
    /// Constant-rate emitters give off particles until this age.
    until: f32,
    /// Each emitter's fraction of a particle owed.
    owed: Vec<f32>,
    particles: Vec<Particle>,
}

/// Effects running in the world.
#[derive(Debug, Clone, Default)]
pub struct Particles {
    instances: Vec<Instance>,
    seed: u32,
    /// Effects started, by hash, since the start.
    pub started: HashMap<u32, u32>,
}

impl Particles {
    /// Starts `effect` at `at`, `scale` times its size; constant-rate
    /// emitters run `seconds` ([`ONE_OFF`] if 0). False if the library
    /// has no such effect.
    pub fn spawn(
        &mut self,
        lib: &Library,
        effect: u32,
        at: Vec3,
        seconds: f32,
        scale: f32,
    ) -> bool {
        self.start(
            lib,
            effect,
            at,
            if seconds > 0.0 { seconds } else { ONE_OFF },
            scale,
        )
    }

    /// Starts `effect` at `at`, `scale` times its size, for as long as its
    /// template's record says ([`EffectRun`]): giving off particles for
    /// its `Duration`, and for one that deletes itself no longer than its
    /// own life, `Duration` give or take `DurationVar`.
    pub fn spawn_run(
        &mut self,
        lib: &Library,
        effect: u32,
        at: Vec3,
        run: EffectRun,
        scale: f32,
    ) -> bool {
        let until = if run.auto_delete {
            let life = run.duration + self.spread(run.var);
            life.min(run.duration)
        } else {
            run.duration
        };
        self.start(lib, effect, at, until.max(0.0), scale)
    }

    fn start(&mut self, lib: &Library, effect: u32, at: Vec3, until: f32, scale: f32) -> bool {
        let Some(spec) = lib.effects.get(&effect) else {
            return false;
        };
        *self.started.entry(effect).or_default() += 1;
        self.instances.push(Instance {
            effect,
            key: None,
            at,
            scale,
            age: 0.0,
            until,
            owed: vec![0.0; spec.emitters.len()],
            particles: Vec::new(),
        });
        true
    }

    /// Keeps `effect` going at `at` for source `key` (a fire, a smoking
    /// wreck): started on the first call, moved to `at` and kept up for
    /// [`HOLD`] more seconds on each later one. A held effect's keyed
    /// rates start over each time they run out.
    pub fn hold(&mut self, lib: &Library, key: u64, effect: u32, at: Vec3, scale: f32) -> bool {
        if let Some(i) = self
            .instances
            .iter_mut()
            .find(|i| i.key == Some(key) && i.effect == effect)
        {
            i.at = at;
            i.until = i.age + HOLD;
            return true;
        }
        if !self.spawn(lib, effect, at, HOLD, scale) {
            return false;
        }
        if let Some(i) = self.instances.last_mut() {
            i.key = Some(key);
        }
        true
    }

    /// Starts every effect template `id` shows; how many it started. With
    /// `seconds` 0, each runs as long as the template's record says
    /// ([`spawn_run`](Self::spawn_run)), where the library knows it.
    pub fn spawn_template(
        &mut self,
        lib: &Library,
        id: u32,
        at: Vec3,
        seconds: f32,
        scale: f32,
    ) -> usize {
        let fx: Vec<u32> = lib.of_template(id).to_vec();
        fx.into_iter()
            .enumerate()
            .filter(|&(k, h)| match lib.run(id, k).filter(|_| seconds <= 0.0) {
                Some(run) => self.spawn_run(lib, h, at, run, scale),
                None => self.spawn(lib, h, at, seconds, scale),
            })
            .count()
    }

    /// Particles alive.
    pub fn live(&self) -> usize {
        self.instances.iter().map(|i| i.particles.len()).sum()
    }

    /// Effects still running.
    pub fn running(&self) -> usize {
        self.instances.len()
    }

    fn random(&mut self) -> f32 {
        // xorshift: cheap, and the same run to run.
        let mut x = self.seed.wrapping_add(0x9e37_79b9);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Plus or minus `var`.
    fn spread(&mut self, var: f32) -> f32 {
        (self.random() * 2.0 - 1.0) * var
    }

    /// Runs every effect `dt` seconds on.
    pub fn step(&mut self, lib: &Library, dt: f32) {
        let mut total = self.live();
        let mut instances = std::mem::take(&mut self.instances);
        for inst in &mut instances {
            let Some(spec) = lib.effects.get(&inst.effect) else {
                inst.particles.clear();
                continue;
            };
            inst.age += dt;
            let frame = inst.age * FRAME;
            // The effect's forces this frame ([`ForceSpec::live`]).
            let live: Vec<LiveForce> = spec
                .forces
                .iter()
                .map(|f| f.live(frame, inst.at, inst.scale))
                .collect();
            // Move and age what is there, by the effect's forces ([`push`]).
            for p in &mut inst.particles {
                let e = &spec.emitters[usize::from(p.emitter)];
                p.age += dt;
                (p.velocity, p.at) = push(&live, e, p.at, p.velocity, p.draw, dt);
                p.turn += p.spin * dt;
            }
            inst.particles.retain(|p| p.age < p.life);
            // Give off new ones.
            for (k, e) in spec.emitters.iter().enumerate() {
                let held = inst.key.is_some();
                let giving = if e.bursts() && !held {
                    inst.age <= e.burst_end()
                } else {
                    inst.age <= inst.until
                };
                if !giving || e.frames.is_empty() {
                    continue;
                }
                let clock = match (held, e.burst_end() * FRAME) {
                    (true, end) if end > 0.0 => frame % end,
                    _ => frame,
                };
                let rate = (e.rate.at(clock) + self.spread(e.rate_var)).max(0.0);
                inst.owed[k] += rate * dt;
                let mut given = 0;
                while inst.owed[k] >= 1.0 {
                    inst.owed[k] -= 1.0;
                    if given >= EMITTER_BUDGET || total >= BUDGET {
                        continue;
                    }
                    given += 1;
                    let p = self.emit(e, k as u16, inst.at, inst.scale, frame);
                    inst.particles.push(p);
                    total += 1;
                }
            }
        }
        // Done once nothing more will come and nothing is left.
        instances.retain(|inst| {
            let Some(spec) = lib.effects.get(&inst.effect) else {
                return false;
            };
            let more = spec.emitters.iter().any(|e| {
                if inst.key.is_some() {
                    inst.age <= inst.until
                } else if e.bursts() {
                    inst.age <= e.burst_end()
                } else {
                    inst.age <= inst.until && e.rate.at(0.0) > 0.0
                }
            });
            more || !inst.particles.is_empty()
        });
        self.instances = instances;
    }

    fn emit(
        &mut self,
        e: &EmitterSpec,
        emitter: u16,
        at: Vec3,
        scale: f32,
        frame: f32,
    ) -> Particle {
        let half = (e.spread.at(frame) + self.spread(e.spread_var)).to_radians();
        // A direction tilted from the axis by an angle even in 0 to `half`
        // (so crowding the axis, not even over the cap), then turned about
        // it by an even angle. From the 360 code (0x821D7880: two
        // quaternions from its sine table, the tilt `random x spread`, the
        // turn `random x 2 pi`).
        let tilt = self.random() * half;
        let (sin, cos) = tilt.sin_cos();
        let around = self.random() * std::f32::consts::TAU;
        let local = Vec3::new(sin * around.cos(), cos, sin * around.sin());
        // On its shape, flying out along the patch's normal (the patch's
        // own space turned as the emitter is); with no shape, at the
        // emitter, along its axis.
        let (start, axis) = match self.on_shape(e) {
            Some((p, n)) => (e.pose * p, e.pose * n),
            None => (Vec3::ZERO, e.axis),
        };
        let dir = Quat::from_rotation_arc(Vec3::Y, axis.normalize_or(Vec3::Y)) * local;
        // Each a fresh draw on its spread, a speed not held above zero, a
        // life held at or above zero (from the 360 code: 0x821DF920,
        // 0x821DAE60).
        let speed = e.speed.at(frame) + self.spread(e.speed_var);
        let life = (e.life.at(frame) + self.spread(e.life_var)).max(0.0);
        Particle {
            at: at + (e.offset + start) * scale,
            velocity: dir * speed * scale,
            age: 0.0,
            life,
            scale,
            turn: self.spread(e.turn * 0.5).to_radians(),
            spin: e.spin.to_radians(),
            emitter,
            draw: (0.5 - (self.random() * 256.0).floor().min(255.0) / 255.0) * 2.0,
        }
    }

    /// A point on emitter `e`'s shape, in the shape's own space, and the
    /// normal of the patch it is on: patches by their area, points evenly
    /// within one. `None` for an emitter without a shape.
    fn on_shape(&mut self, e: &EmitterSpec) -> Option<(Vec3, Vec3)> {
        let total = e.shape.last()?.up_to;
        let pick = self.random() * total;
        let patch = e
            .shape
            .iter()
            .find(|p| p.up_to >= pick)
            .or(e.shape.last())?;
        // Even over a triangle: the square root keeps the far corners
        // from crowding.
        let r1 = self.random().sqrt();
        let r2 = self.random();
        let [a, b, c] = patch.corners;
        Some((
            a * (1.0 - r1) + b * (r1 * (1.0 - r2)) + c * (r1 * r2),
            patch.normal,
        ))
    }

    /// Every live particle's position, for measurements.
    pub fn positions(&self) -> Vec<Vec3> {
        self.instances
            .iter()
            .flat_map(|i| i.particles.iter().map(|p| p.at))
            .collect()
    }
    /// How far the particles sit from `centre` on average, metres (0 with
    /// none): a measure of an effect's spread, for the logs.
    pub fn mean_distance(&self, centre: Vec3) -> f32 {
        let all: Vec<f32> = self
            .instances
            .iter()
            .flat_map(|i| i.particles.iter().map(|p| p.at.distance(centre)))
            .collect();
        if all.is_empty() {
            0.0
        } else {
            all.iter().sum::<f32>() / all.len() as f32
        }
    }

    /// Every particle as a sprite.
    pub fn sprites(&self, lib: &Library, out: &mut Vec<Sprite>) {
        for inst in &self.instances {
            let Some(spec) = lib.effects.get(&inst.effect) else {
                continue;
            };
            for p in &inst.particles {
                let e = &spec.emitters[usize::from(p.emitter)];
                if let Some(s) = sprite(e, p.at, p.age / p.life, p.scale, p.draw, p.turn) {
                    out.push(s);
                }
            }
        }
    }
}

/// Emitter `e`'s particle `t` (0 to 1) through its life, its draw `draw`,
/// its effect drawn `scale` times its size.
///
/// From the 360 code (its sprite fill, 0x821E3F38): the width is the size
/// keys at `99 t` plus the size spread times the particle's draw
/// (0x821DFEF0); the colour runs between the key at `99 t` and the next
/// by the fraction between, in steps of 1/255, the brightness factor
/// that of the lower key (0x821E2C98).
fn sprite(e: &EmitterSpec, at: Vec3, t: f32, scale: f32, draw: f32, turn: f32) -> Option<Sprite> {
    let t = t.clamp(0.0, 1.0);
    let n = e.frames.len();
    let uv = *e
        .frames
        .get(((t * n as f32) as usize).min(n.checked_sub(1)?))?;
    let size = (e.size.at(t * 99.0) + draw * e.size_var) * scale;
    let last = e.colour.len().checked_sub(1)?;
    let at99 = t * 99.0;
    let i = (at99 as usize).min(last);
    let key = e.colour.get(i)?;
    let next = e.colour.get((i + 1).min(last))?;
    let step = ((at99 - i as f32) * 255.0) as i32;
    let blend = |a: u8, b: u8| ((((i32::from(b) - i32::from(a)) * step) >> 8) + i32::from(a)) as u8;
    let ([r0, g0, b0, a0], [r1, g1, b1, a1]) = (key.rgba(), next.rgba());
    let [r, g, b, a] = [blend(r0, r1), blend(g0, g1), blend(b0, b1), blend(a0, a1)];
    let gain = key.intensity.abs();
    // A glow's colour stays a display value, as the game's particle shader
    // takes it (346: the key's colour times its brightness factor): the
    // renderer adds glows in display values (its glow layer).
    let glow = key.intensity < 0.0;
    let lin = |c: u8| {
        if glow {
            f32::from(c) / 255.0 * gain
        } else {
            srgb_to_linear(f32::from(c) / 255.0) * gain
        }
    };
    Some(Sprite {
        at,
        size,
        turn,
        uv,
        colour: [lin(r), lin(g), lin(b), f32::from(a) / 255.0],
        glow,
    })
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(keys: &[(f32, f32)]) -> Attr {
        Attr {
            keys: keys.to_vec(),
            ..zero(0)
        }
    }

    fn plain(v: f32) -> Attr {
        Attr {
            value: v.to_le_bytes(),
            ..zero(0)
        }
    }

    fn key(rgba: [u8; 4], intensity: f32) -> ColourKey {
        let [r, g, b, a] = rgba;
        ColourKey {
            bgra: [b, g, r, a],
            intensity,
            unknown_6: 0.0,
        }
    }

    /// A grenade-like burst: 300 a second for the first 6 frames (0.1 s at
    /// [`FRAME`]), then none; particles live 1 s.
    fn burst() -> EmitterSpec {
        EmitterSpec {
            offset: Vec3::Y,
            pose: Quat::IDENTITY,
            axis: Vec3::Y,
            shape: Vec::new(),
            size: keyed(&[(0.0, 1.0), (99.0, 3.0)]),
            size_var: 0.0,
            life: plain(1.0),
            life_var: 0.0,
            rate: keyed(&[(0.0, 300.0), (6.0, 300.0), (6.0, 0.0)]),
            rate_var: 0.0,
            speed: plain(0.0),
            speed_var: 0.0,
            spread: plain(0.0),
            spread_var: 0.0,
            mass: Spread {
                value: 1.0,
                var: 0.0,
            },
            gravity: Spread::default(),
            drag: Spread::default(),
            wind: Spread::default(),
            attract: Spread::default(),
            swirl: Spread::default(),
            turn: 0.0,
            spin: 0.0,
            frames: vec![[0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 0.5, 0.5]],
            textures: 2,
            found: 2,
            colour: (0..100).map(|_| key([255, 128, 0, 255], -10.0)).collect(),
        }
    }

    fn library(e: EmitterSpec) -> Library {
        library_with(e, Vec::new())
    }

    fn force(kind: ForceKind, strength: f32) -> ForceSpec {
        ForceSpec {
            kind,
            tagged: true,
            strength: plain(strength),
            radius: plain(0.0),
            range: plain(0.0),
            decay: plain(0.0),
            radial: plain(0.0),
            local: plain(0.0),
            position: [plain(0.0), plain(0.0), plain(0.0)],
            turn: [plain(0.0), plain(0.0), plain(0.0)],
        }
    }

    /// The per-frame setup as the 360 code has it: a gravity points down
    /// its frame (turned 90 degrees about z, along +x), others up; a drag's
    /// strength is strength x pi x radius squared; a wind takes the
    /// world's wind, of which the alpha has none.
    #[test]
    fn each_kinds_frame_setup_follows_the_360_code() {
        let mut g = force(ForceKind::Gravity, 4.0);
        let live = g.live(0.0, Vec3::new(1.0, 2.0, 3.0), 1.0);
        assert!(live.direction.abs_diff_eq(Vec3::NEG_Y, 1e-5));
        assert_eq!(live.at, Vec3::new(1.0, 2.0, 3.0));
        g.turn[2] = plain(90.0);
        assert!(
            g.live(0.0, Vec3::ZERO, 1.0)
                .direction
                .abs_diff_eq(Vec3::X, 1e-5)
        );
        let up = force(ForceKind::Attractor, 1.0).live(0.0, Vec3::ZERO, 1.0);
        assert!(up.direction.abs_diff_eq(Vec3::Y, 1e-5));
        let mut d = force(ForceKind::Drag, 20.0);
        d.radius = plain(0.1);
        let k = d.live(0.0, Vec3::ZERO, 1.0).strength;
        assert!((k - 20.0 * std::f32::consts::PI * 0.01).abs() < 1e-5, "{k}");
        assert_eq!(
            force(ForceKind::Wind, 5.0)
                .live(0.0, Vec3::ZERO, 1.0)
                .strength,
            0.0
        );
        // Keyed settings are read at the effect's time.
        let mut timed = force(ForceKind::Gravity, 0.0);
        timed.strength = keyed(&[(0.0, 0.0), (10.0, 10.0)]);
        assert!((timed.live(5.0, Vec3::ZERO, 1.0).strength - 5.0).abs() < 1e-5);
    }

    fn spread(value: f32) -> Spread {
        Spread { value, var: 0.0 }
    }

    /// The 360 particle step (0x827DD6A0) for each kind, worked by hand:
    /// mass 2, `dt` 0.1, a particle 2 m out along x moving 4 m/s along z.
    #[test]
    fn each_kind_moves_a_particle_as_the_360_step_does() {
        let mut e = burst();
        e.mass = spread(2.0);
        e.gravity = spread(0.5);
        e.drag = spread(1.0);
        e.wind = spread(0.5);
        e.attract = spread(1.0);
        e.swirl = spread(1.0);
        let dt = 0.1;
        let at = Vec3::new(2.0, 0.0, 0.0);
        let v = Vec3::new(0.0, 0.0, 4.0);
        let live = |f: ForceSpec| f.live(0.0, Vec3::ZERO, 1.0);
        let step = |f: &[LiveForce], at: Vec3| push(f, &e, at, v, 0.0, dt).0;
        let near = |a: Vec3, b: Vec3| a.abs_diff_eq(b, 1e-4);
        // No forces: unchanged, and the place moves on by v dt.
        assert_eq!(push(&[], &e, at, v, 0.0, dt), (v, at + v * dt));
        // Gravity: down x 0.5 x 10 x mass 2 = 10 N on 2 kg.
        let g = step(&[live(force(ForceKind::Gravity, 10.0))], at);
        assert!(near(g, Vec3::new(0.0, -0.5, 4.0)), "{g}");
        // Drag 20 pi (radius 1) would stop it past still: cut to m / (s dt)
        // = 5, so v + (-5 x 4 x v) x 0.05 / 1.25 = v / 5.
        let mut drag = force(ForceKind::Drag, 20.0);
        drag.radius = plain(1.0);
        assert!(near(step(&[live(drag)], at), v * 0.2));
        // A light drag: k = 0.01 pi, uncut.
        let mut light = force(ForceKind::Drag, 1.0);
        light.radius = plain(0.1);
        let k = 0.01 * std::f32::consts::PI;
        let want = v + (-v * k * 4.0) * 0.05 / (1.0 + k * 0.05);
        assert!(near(step(&[live(light.clone())], at), want));
        // Wind: along its way by strength 4 x 0.5 = 2 N; its square root
        // (2) doubles the light drag.
        let mut wind = live(force(ForceKind::Wind, 0.0));
        wind.direction = Vec3::X;
        wind.strength = 4.0;
        assert!(near(step(&[wind], at), v + Vec3::X * 0.1));
        let k2 = 2.0 * k;
        let want = v + (Vec3::X * 2.0 - v * k2 * 4.0) * 0.05 / (1.0 + k2 * 0.05);
        assert!(near(step(&[wind, live(light)], at), want));
        // A local attractor, strength 3, range 4, decay 1, 2 m off: pushed
        // out by r x 3 x (1 - 2/4) / (2^3 + 1); nothing past its range; a
        // decay of -1 fades by (d/R)^1 = 0.5 instead (0 less the decay).
        let mut pull = force(ForceKind::Attractor, 3.0);
        pull.range = plain(4.0);
        pull.decay = plain(1.0);
        pull.local = plain(1.0);
        let a = step(&[live(pull.clone())], at);
        assert!(near(a, v + Vec3::X * (2.0 * 1.5 / 9.0 * 0.05)), "{a}");
        assert_eq!(step(&[live(pull.clone())], Vec3::X * 5.0), v);
        pull.decay = plain(-1.0);
        let a = step(&[live(pull.clone())], at);
        assert!(near(a, v + Vec3::X * (2.0 * 3.0 * 0.5 / 9.0 * 0.05)), "{a}");
        // Not local: r x 3 / (d^2 + 1), no range.
        pull.local = plain(0.0);
        let a = step(&[live(pull)], at);
        assert!(near(a, v + Vec3::X * (2.0 * 3.0 / 5.0 * 0.05)), "{a}");
        // A vortex, not local: (radial r/d + r x v) x 3 / 9; r x v =
        // (0, -8, 0).
        let mut swirl = force(ForceKind::Vortex, 3.0);
        swirl.radial = plain(1.0);
        let s = step(&[live(swirl.clone())], at);
        assert!(near(s, v + Vec3::new(1.0, -8.0, 0.0) / 3.0 * 0.05), "{s}");
        // Local, range 4, decay 1, the particle 2 m up y: strength 8 x
        // fade 0.5 = 4 turns v about x, w x v = (4, 0, 0) x (0, 0, 4) =
        // (0, -16, 0); radial 1 pushes out along y by 5.
        swirl.local = plain(1.0);
        swirl.range = plain(4.0);
        swirl.decay = plain(1.0);
        swirl.strength = plain(8.0);
        let up = Vec3::new(0.0, 2.0, 0.0);
        let s = step(&[live(swirl.clone())], up);
        assert!(near(s, v + Vec3::new(0.0, 5.0 - 16.0, 0.0) * 0.05), "{s}");
        // Strength x fade 1.5: the 360 select by the bits of 2^32 leaves
        // about 1e-29, so no turn, only the push out.
        swirl.strength = plain(3.0);
        let s = step(&[live(swirl)], up);
        assert!(near(s, v + Vec3::new(0.0, 5.0, 0.0) * 0.05), "{s}");
        assert_eq!(masked(4.0, true), 4.0);
        assert!(masked(1.5, true) < 1e-20);
        assert!(masked(2.0, false).abs() < 1e-20);
    }

    /// Each particle takes its own draw on every spread.
    #[test]
    fn spreads_take_the_particle_s_draw() {
        let s = Spread {
            value: 2.0,
            var: 0.5,
        };
        assert_eq!(s.at(1.0), 2.5);
        assert_eq!(s.at(-1.0), 1.5);
        let lib = library(burst());
        let mut p = Particles::default();
        p.spawn(&lib, 7, Vec3::ZERO, 0.0, 1.0);
        p.step(&lib, 0.1);
        let draws: Vec<f32> = p.instances[0].particles.iter().map(|q| q.draw).collect();
        assert!(draws.iter().all(|d| (-1.0..=1.0).contains(d)));
        assert!(draws.iter().any(|d| *d > 0.2) && draws.iter().any(|d| *d < -0.2));
    }

    /// An effect with forces moves by them.
    #[test]
    fn an_effect_with_forces_moves_by_them() {
        let mut e = burst();
        e.gravity = Spread {
            value: 1.0,
            var: 0.0,
        };
        e.speed = plain(0.0);
        let lib = library_with(e.clone(), vec![force(ForceKind::Gravity, -2.0)]);
        let mut p = Particles::default();
        p.spawn(&lib, 7, Vec3::ZERO, 0.0, 1.0);
        for _ in 0..10 {
            p.step(&lib, 0.05);
        }
        // Strength -2 along down: up at 2 m/s2.
        let q = &p.instances[0].particles[0];
        assert!(
            q.velocity.y > 0.0 && (q.velocity.y - 2.0 * 0.5).abs() < 0.11,
            "{:?}",
            q.velocity
        );
    }

    fn library_with(e: EmitterSpec, forces: Vec<ForceSpec>) -> Library {
        Library {
            effects: HashMap::from([(
                7,
                EffectSpec {
                    hash: 7,
                    emitters: vec![e],
                    forces,
                },
            )]),
            by_template: HashMap::from([(0x8000_0001, vec![7, 99])]),
            ..Library::default()
        }
    }

    /// An emitter with a shape of two patches, one four times the other's
    /// area and 10 m up, turned a quarter about Y: particles start on the
    /// patches (turned), four in five on the big one, and fly out along
    /// the patch's turned normal.
    #[test]
    fn particles_start_on_their_shape_by_area_and_leave_along_its_normal() {
        let mut e = burst();
        e.offset = Vec3::ZERO;
        e.speed = plain(2.0);
        e.pose = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        // The small patch lies in the XZ plane at the origin, normal +Y;
        // the big one in the XY plane 10 m up, normal +Z.
        e.shape = vec![
            Patch {
                corners: [Vec3::ZERO, Vec3::X, Vec3::Z],
                normal: Vec3::Y,
                up_to: 0.5,
            },
            Patch {
                corners: [
                    Vec3::new(0.0, 10.0, 0.0),
                    Vec3::new(2.0, 10.0, 0.0),
                    Vec3::new(0.0, 12.0, 0.0),
                ],
                normal: Vec3::Z,
                up_to: 2.5,
            },
        ];
        let lib = library(e);
        let mut p = Particles::default();
        p.spawn(&lib, 7, Vec3::new(100.0, 0.0, 0.0), 0.0, 1.0);
        p.step(&lib, 0.1);
        let (mut small, mut big) = (0, 0);
        for q in &p.instances[0].particles {
            let local = Vec3::new(100.0, 0.0, 0.0) - q.at;
            if q.at.y < 1.0 {
                small += 1;
                // Turned a quarter about Y, the XZ patch stays in XZ,
                // within its triangle's reach, flying up.
                assert!(local.length() <= 1.0 + 1e-3, "{:?}", q.at);
                assert!(
                    q.velocity.abs_diff_eq(Vec3::Y * 2.0, 1e-3),
                    "{:?}",
                    q.velocity
                );
            } else {
                big += 1;
                // X turned onto -Z: the patch spans z in -2..0 and y in
                // 10..12, and its +Z normal turned onto +X.
                assert!((10.0..=12.0).contains(&q.at.y), "{:?}", q.at);
                assert!((-2.0 - 1e-3..=1e-3).contains(&(q.at.z)), "{:?}", q.at);
                assert!(
                    q.velocity.abs_diff_eq(Vec3::X * 2.0, 1e-3),
                    "{:?}",
                    q.velocity
                );
            }
        }
        assert_eq!(small + big, 30);
        assert!(big > 18 && small > 1, "{small} small, {big} big");
        assert!(p.mean_distance(Vec3::new(100.0, 0.0, 0.0)) > 7.0);
    }

    /// The 360 cone: the tilt from the axis is even in 0 to the spread, so
    /// over a 90-degree spread it averages 45 degrees (even over the cap
    /// it would average 57).
    #[test]
    fn particles_tilt_evenly_up_to_their_spread() {
        let mut e = burst();
        e.speed = plain(1.0);
        e.spread = plain(90.0);
        let mut p = Particles::default();
        let n = 4000;
        let mean: f32 = (0..n)
            .map(|_| {
                p.emit(&e, 0, Vec3::ZERO, 1.0, 0.0)
                    .velocity
                    .angle_between(Vec3::Y)
            })
            .sum::<f32>()
            / n as f32;
        assert!(
            (mean.to_degrees() - 45.0).abs() < 1.5,
            "{}",
            mean.to_degrees()
        );
    }

    #[test]
    fn a_burst_gives_off_its_keyed_rate_then_ends() {
        let lib = library(burst());
        let mut p = Particles::default();
        assert!(p.spawn(&lib, 7, Vec3::ZERO, 0.0, 1.0));
        assert!(!p.spawn(&lib, 8, Vec3::ZERO, 0.0, 1.0));
        let dt = 1.0 / 60.0;
        for _ in 0..12 {
            p.step(&lib, dt);
        }
        // 300 a second over 6 frames (0.1 s), with one step's rounding.
        assert!((29..=31).contains(&p.live()), "{}", p.live());
        for _ in 0..80 {
            p.step(&lib, dt);
        }
        assert_eq!(p.live(), 0);
        assert_eq!(p.running(), 0);
    }

    /// As the 360 fills a sprite (0x821E3F38): the size spread adds its
    /// draw in metres; the colour runs between neighbouring keys, the
    /// brightness factor that of the lower key.
    #[test]
    fn sprites_take_their_spread_in_metres_and_blend_their_keys() {
        let mut e = burst();
        e.size = plain(7.0);
        e.size_var = 2.0;
        e.colour = (0..100)
            .map(|i| {
                key(
                    [if i % 2 == 0 { 0 } else { 200 }, 0, 0, 255],
                    -(i as f32) - 1.0,
                )
            })
            .collect();
        let at = |t: f32, draw: f32| sprite(&e, Vec3::ZERO, t, 1.0, draw, 0.0).unwrap();
        assert_eq!(at(0.0, 1.0).size, 9.0);
        assert_eq!(at(0.0, -1.0).size, 5.0);
        assert_eq!(at(0.0, 0.5).size, 8.0);
        // Half-way from key 10 (red 0) to key 11 (red 200): 127 / 256 of
        // the way, the factor key 10's (11).
        let s = at(10.5 / 99.0, 0.0);
        let red = (200 * 127 / 256) as f32 / 255.0 * 11.0;
        assert!((s.colour[0] - red).abs() < 0.05, "{:?}", s.colour);
    }

    #[test]
    fn sprites_flip_through_frames_and_follow_the_keys() {
        let e = burst();
        let start = sprite(&e, Vec3::ZERO, 0.0, 1.0, 0.0, 0.0).unwrap();
        let end = sprite(&e, Vec3::ZERO, 0.99, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(start.uv, [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(end.uv, [0.5, 0.0, 0.5, 0.5]);
        assert_eq!(start.size, 1.0);
        assert!((end.size - 2.98).abs() < 0.01);
        // Negative brightness: a glow, its colour ten times over, kept in
        // display values.
        assert!(start.glow);
        assert!((start.colour[0] - 10.0).abs() < 1e-4);
        assert!((start.colour[1] - 10.0 * 128.0 / 255.0).abs() < 1e-4);
        assert_eq!(start.colour[3], 1.0);
        assert!(e.glows());
    }

    #[test]
    fn steady_emitters_run_as_long_as_asked_and_their_forces_lift_them() {
        let mut e = burst();
        e.rate = plain(20.0);
        e.life = plain(4.0);
        e.gravity = Spread {
            value: 1.0,
            var: 0.0,
        };
        e.colour = (0..100).map(|_| key([40, 40, 40, 150], 1.0)).collect();
        assert!(!e.glows());
        // A gravity of strength -1.5 lifts at 1.5 m/s2.
        let lib = library_with(e, vec![force(ForceKind::Gravity, -1.5)]);
        let mut p = Particles::default();
        p.spawn(&lib, 7, Vec3::ZERO, 2.0, 1.0);
        let dt = 0.05;
        for _ in 0..40 {
            p.step(&lib, dt);
        }
        // Given off for 2 s at 20 a second; none has lived out its 4 s.
        assert!((39..=41).contains(&p.live()), "{}", p.live());
        let mut s = Vec::new();
        p.sprites(&lib, &mut s);
        assert!(s.iter().all(|s| !s.glow && s.at.y >= 1.0));
        assert!(s.iter().any(|s| s.at.y > 1.5));
        for _ in 0..100 {
            p.step(&lib, dt);
        }
        assert_eq!(p.running(), 0);
    }

    #[test]
    fn a_held_effect_follows_its_source_until_it_stops_calling() {
        let mut e = burst();
        e.rate = plain(10.0);
        let lib = library(e);
        let mut p = Particles::default();
        let dt = 0.1;
        for k in 0..30 {
            assert!(p.hold(&lib, 5, 7, Vec3::X * k as f32, 1.0));
            p.step(&lib, dt);
        }
        assert_eq!(p.running(), 1);
        // It gave off all along, from where its source last was.
        assert!(p.live() >= 9, "{}", p.live());
        let mut s = Vec::new();
        p.sprites(&lib, &mut s);
        assert!(s.iter().any(|s| s.at.x > 28.0));
        // Once the calls stop, it runs down.
        for _ in 0..40 {
            p.step(&lib, dt);
        }
        assert_eq!(p.running(), 0);
    }

    #[test]
    fn a_template_starts_its_known_effects() {
        let lib = library(burst());
        let mut p = Particles::default();
        assert_eq!(p.spawn_template(&lib, 0x8000_0001, Vec3::ZERO, 0.0, 1.0), 1);
        assert_eq!(p.spawn_template(&lib, 0x8000_0002, Vec3::ZERO, 0.0, 1.0), 0);
        assert_eq!(p.started.get(&7), Some(&1));
    }

    // A template's effect gives off for its `Duration`; one that deletes
    // itself for no longer than its life, `Duration` give or take its
    // `DurationVar`.
    #[test]
    fn a_templates_effect_runs_as_long_as_its_record_says() {
        let mut lib = library(burst());
        let run = |duration, var, auto_delete| EffectRun {
            duration,
            var,
            auto_delete,
        };
        lib.runs = HashMap::from([(0x8000_0001, vec![run(25.0, 10.0, true)])]);
        let mut p = Particles::default();
        assert_eq!(lib.run(0x8000_0001, 0), Some(run(25.0, 10.0, true)));
        for _ in 0..50 {
            assert_eq!(p.spawn_template(&lib, 0x8000_0001, Vec3::ZERO, 0.0, 1.0), 1);
        }
        assert!(p.instances.iter().all(|i| (15.0..=25.0).contains(&i.until)));
        assert!(p.instances.iter().any(|i| i.until < 24.0), "some die early");
        // Kept, not deleting itself: its whole `Duration`.
        assert!(p.spawn_run(&lib, 7, Vec3::ZERO, run(100.0, 0.0, false), 1.0));
        assert_eq!(p.instances.last().map(|i| i.until), Some(100.0));
        // Asked for a time, that time.
        p.spawn_template(&lib, 0x8000_0001, Vec3::ZERO, 2.0, 1.0);
        assert_eq!(p.instances.last().map(|i| i.until), Some(2.0));
    }

    #[test]
    fn an_emitter_without_atlas_frames_draws_nothing() {
        let mut e = burst();
        e.frames.clear();
        assert!(sprite(&e, Vec3::ZERO, 0.5, 1.0, 0.0, 0.0).is_none());
    }
}
