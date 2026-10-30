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
//! - the frame rate the keyed settings count in, 30 a second ([`FRAME`]):
//!   at it a grenade gives off about 190 dirt chunks, at one a second
//!   5,800;
//! - `Size` as a particle's half-width in metres (the footage's small
//!   fires stand 1 to 2 m tall; `Size` 0.5 to 1 as a full width drew them
//!   a third of that), `Speed` in metres a second,
//!   `Spread` a cone's half angle in degrees about the emitter's up,
//!   `life` in seconds, each `var` a plus-or-minus spread.
//!
//! Invented (no data decides them yet): how particles move once given off
//! ([`RISE`], [`DRAG`]: emitters' `FRCE` forces are decoded but not
//! applied), how long a constant-rate emitter runs in a one-off effect
//! ([`ONE_OFF`]), which way a particle leaves its shape (along its
//! patch's normal; inferred) and the particle budgets.

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::effect::{
    self, ATLAS_DICT, ATLAS_TYPE, Atlas, Attr, ColourKey, EFFECT_TYPE, Effect, setting,
};
use m2_assets::{Layer, name_hash};
use m2_math::glam::{EulerRot, Quat, Vec3};

use crate::assets::{AssetError, AssetSource};
use crate::tuning::Table;

/// The keyed settings' clock: frames a second (inferred).
pub const FRAME: f32 = 30.0;
/// How long a constant-rate emitter gives off particles when its effect is
/// set off once (an explosion's), seconds. Invented.
pub const ONE_OFF: f32 = 0.5;
/// What lifts light-giving particles and long-lived blended ones (smoke)
/// as their `forcegravity` scales it, m/s². Invented; short-lived
/// blended ones (dirt, debris) fall under gravity instead.
pub const RISE: f32 = 1.5;
/// How fast `forcedrag` 1 slows a particle, per second. Invented.
pub const DRAG: f32 = 2.0;
/// How long a held effect runs past its source's last call, seconds
/// (sources call every 0.3 to 0.4 s). Invented.
pub const HOLD: f32 = 0.6;
/// Blended particles living at least this long rise rather than fall,
/// seconds. Invented.
const SMOKE_LIFE: f32 = 2.5;
const GRAVITY: f32 = 9.81;
/// Most particles one emitter of one effect keeps.
const EMITTER_BUDGET: usize = 400;
/// Most particles in the world.
const BUDGET: usize = 12_000;

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
    pub gravity: f32,
    pub drag: f32,
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
            gravity: num(setting::FORCE_GRAVITY),
            drag: num(setting::FORCE_DRAG),
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
        }
    }
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
    /// Half its width, metres.
    pub size: f32,
    /// Turn about the view direction, radians.
    pub turn: f32,
    /// Atlas rectangle: left, top, width, height.
    pub uv: [f32; 4],
    /// Linear colour, brightness included, and alpha.
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
    /// Its own size factor (1 plus or minus the spread).
    scale: f32,
    turn: f32,
    spin: f32,
    emitter: u16,
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
            // Move and age what is there.
            for p in &mut inst.particles {
                let e = &spec.emitters[usize::from(p.emitter)];
                p.age += dt;
                let lift = if e.glows() || p.life >= SMOKE_LIFE {
                    RISE * e.gravity
                } else {
                    -GRAVITY * e.gravity
                };
                p.velocity.y += lift * dt;
                p.velocity *= (1.0 - e.drag * DRAG * dt).max(0.0);
                p.at += p.velocity * dt;
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
                let mine = inst
                    .particles
                    .iter()
                    .filter(|p| usize::from(p.emitter) == k)
                    .count();
                while inst.owed[k] >= 1.0 {
                    inst.owed[k] -= 1.0;
                    if mine + 1 > EMITTER_BUDGET || total >= BUDGET {
                        continue;
                    }
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
        let half = (e.spread.at(frame) + self.spread(e.spread_var))
            .clamp(0.0, 180.0)
            .to_radians();
        // A direction within `half` of the axis, even over the cap.
        let cos = 1.0 - self.random() * (1.0 - half.cos());
        let sin = (1.0 - cos * cos).max(0.0).sqrt();
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
        let speed = (e.speed.at(frame) + self.spread(e.speed_var)).max(0.0);
        let life = (e.life.at(frame) + self.spread(e.life_var)).max(0.05);
        Particle {
            at: at + (e.offset + start) * scale,
            velocity: dir * speed * scale,
            age: 0.0,
            life,
            scale: (1.0 + self.spread(e.size_var)).max(0.05) * scale,
            turn: self.spread(e.turn * 0.5).to_radians(),
            spin: e.spin.to_radians(),
            emitter,
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
                if let Some(s) = sprite(e, p.at, p.age / p.life, p.scale, p.turn) {
                    out.push(s);
                }
            }
        }
    }
}

/// Emitter `e`'s particle `t` (0 to 1) through its life.
fn sprite(e: &EmitterSpec, at: Vec3, t: f32, scale: f32, turn: f32) -> Option<Sprite> {
    let t = t.clamp(0.0, 1.0);
    let n = e.frames.len();
    let uv = *e
        .frames
        .get(((t * n as f32) as usize).min(n.checked_sub(1)?))?;
    let size = e.size.at(t * 99.0) * scale;
    let key = e
        .colour
        .get(((t * 99.0) as usize).min(e.colour.len().checked_sub(1)?))?;
    let [r, g, b, a] = key.rgba();
    let gain = key.intensity.abs();
    let lin = |c: u8| srgb_to_linear(f32::from(c) / 255.0) * gain;
    Some(Sprite {
        at,
        size,
        turn,
        uv,
        colour: [lin(r), lin(g), lin(b), f32::from(a) / 255.0],
        glow: key.intensity < 0.0,
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

    /// A grenade-like burst: 300 a second for the first 3 frames, then
    /// none; particles live 1 s.
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
            rate: keyed(&[(0.0, 300.0), (3.0, 300.0), (3.0, 0.0)]),
            rate_var: 0.0,
            speed: plain(0.0),
            speed_var: 0.0,
            spread: plain(0.0),
            spread_var: 0.0,
            gravity: 0.0,
            drag: 0.0,
            turn: 0.0,
            spin: 0.0,
            frames: vec![[0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 0.5, 0.5]],
            textures: 2,
            found: 2,
            colour: (0..100).map(|_| key([255, 128, 0, 255], -10.0)).collect(),
        }
    }

    fn library(e: EmitterSpec) -> Library {
        Library {
            effects: HashMap::from([(
                7,
                EffectSpec {
                    hash: 7,
                    emitters: vec![e],
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
        // 300 a second over 3 frames (0.1 s), with one step's rounding.
        assert!((29..=31).contains(&p.live()), "{}", p.live());
        for _ in 0..80 {
            p.step(&lib, dt);
        }
        assert_eq!(p.live(), 0);
        assert_eq!(p.running(), 0);
    }

    #[test]
    fn sprites_flip_through_frames_and_follow_the_keys() {
        let e = burst();
        let start = sprite(&e, Vec3::ZERO, 0.0, 1.0, 0.0).unwrap();
        let end = sprite(&e, Vec3::ZERO, 0.99, 1.0, 0.0).unwrap();
        assert_eq!(start.uv, [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(end.uv, [0.5, 0.0, 0.5, 0.5]);
        assert_eq!(start.size, 1.0);
        assert!((end.size - 2.98).abs() < 0.01);
        // Negative brightness: a glow, its colour ten times over.
        assert!(start.glow);
        assert!((start.colour[0] - 10.0).abs() < 1e-4);
        assert!((start.colour[1] - 10.0 * srgb_to_linear(128.0 / 255.0)).abs() < 1e-4);
        assert_eq!(start.colour[3], 1.0);
        assert!(e.glows());
    }

    #[test]
    fn steady_emitters_run_as_long_as_asked_and_smoke_rises() {
        let mut e = burst();
        e.rate = plain(20.0);
        e.life = plain(4.0);
        e.gravity = 1.0;
        e.colour = (0..100).map(|_| key([40, 40, 40, 150], 1.0)).collect();
        assert!(!e.glows());
        let lib = library(e);
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
        assert!(sprite(&e, Vec3::ZERO, 0.5, 1.0, 0.0).is_none());
    }
}
