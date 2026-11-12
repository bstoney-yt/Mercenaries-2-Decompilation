//! The game's own particle effects in the alpha: blasts, fires and smoke
//! drawn from the particle atlas with the game's emitters
//! ([`m2_engine::particles`]), in place of the coloured spheres
//! ([`crate::combat::Effect`]) the fight drew them with.
//!
//! The fight says what goes off where ([`GameFx`]); this sets off each
//! template's effects, runs them, and draws every particle as a square
//! facing the camera, cut from the atlas, in two batches: blended (smoke,
//! dust, dirt), back to front, and added (fire, flashes, sparks). Blended
//! particles are drawn unlit, their colour keys' brightness times
//! [`SHADE`] standing in for the sun (invented).
//!
//! Every sprite is sorted back to front, whatever its blend, and drawn
//! in runs of one blend: a fireball's smoke behind its glow where it is
//! behind, over it where it is in front (the game's own particle draw
//! order is not read; by depth is the plain reading). Before, every glow
//! drew after every blended sprite, so no smoke ever crossed a fireball.
//!

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

use anyhow::Context as _;
use m2_assets::effect::ATLAS_TEXTURE;
use m2_engine::AssetSource;
use m2_engine::model::{MaterialCache, upload_texture};
use m2_engine::particles::{HOLD, Library, Particles, Sprite};
use m2_engine::world::Templates;
use m2_math::glam::{Mat4, Vec3};
use m2_render::{
    BlendMode, BufferUsage, DrawCall, DrawQueue, Geometry, IndexBufferDesc, IndexBufferHandle,
    IndexFormat, Layer, MaterialDesc, MaterialId, PointLight, Primitive, RenderBackend, Topology,
    VertexBufferDesc, VertexBufferHandle, VertexFormat, VertexLayout, VertexSemantic, View,
};

use crate::combat::{FxTemplate, GameFx};

/// Most particles drawn in a frame.
const CAPACITY: usize = 12_000;
/// Floats a vertex: place, atlas position, colour.
const VERTEX: usize = 3 + 2 + 4;
/// What blended particles' colour is scaled by, for want of their
/// lighting (invented: the keys' grey smoke read dark against the
/// footage's at 1, washed out at 1.2).
const SHADE: f32 = 0.85;

/// How a template's effects resolved, for the log and the round's
/// numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub asked: u32,
    pub effects: usize,
    pub emitters: usize,
    /// Emitters whose every texture is on the atlas.
    pub textured: usize,
    pub glowing: usize,
}

#[derive(Debug)]
pub struct ParticleDraw {
    lib: Library,
    sim: Particles,
    vb: VertexBufferHandle,
    ib: IndexBufferHandle,
    blended: MaterialId,
    glowing: MaterialId,
    sprites: Vec<Sprite>,
    /// Draw calls the last frame took: the runs of one blend, for the logs.
    pub draw_runs: usize,
    vertices: Vec<f32>,
    /// Templates asked for by name, resolved to ids.
    ids: HashMap<String, Option<u32>>,
    /// Template ids by their names' hashes, once one is asked for so.
    by_hash: Option<HashMap<u32, u32>>,
    /// Per template asked for (its name, or its id in hex).
    pub resolved: HashMap<String, Resolved>,
    /// The lights the effects lit ([`m2_engine::lights`]).
    flashes: Vec<Flash>,
    /// Seconds run, and when each held state effect was last asked for
    /// ([`fresh_hold`]).
    clock: f32,
    held_asks: HashMap<u64, f32>,
}

/// A light an effect lit: where, what, how old, and for a held effect
/// (a fire) the source's key and how long since it last called.
#[derive(Debug)]
struct Flash {
    at: Vec3,
    spec: m2_engine::lights::LightSpec,
    age: f32,
    key: Option<u64>,
    since_call: f32,
}

impl ParticleDraw {
    /// Reads every effect and the atlas, and uploads the atlas.
    pub fn load(
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        source: &mut AssetSource<BufReader<File>>,
        templates: &Templates,
    ) -> anyhow::Result<Self> {
        let mut lib = Library::load(source, templates.effects.clone()).context("effects")?;
        lib.runs = templates.effect_runs.clone();
        let atlas = source
            .texture(ATLAS_TEXTURE, 2048)
            .context("the particle atlas")?;
        let (texture, _) = upload_texture(backend, &atlas)?;
        let mut define = |blend| {
            materials.define(
                backend,
                &MaterialDesc {
                    texture: Some(texture),
                    color: [1.0; 4],
                    blend,
                    double_sided: true,
                    lit: false,
                    vertex_color: true,
                    fog: true,
                    clamp: true,
                    ..MaterialDesc::default()
                },
            )
        };
        let blended = define(BlendMode::Translucent)?;
        let glowing = define(BlendMode::Additive)?;
        use VertexFormat::*;
        use VertexSemantic::*;
        let layout =
            VertexLayout::packed(&[(Position, Float3), (TexCoord(0), Float2), (Color, Float4)])?;
        let vb = backend.create_vertex_buffer(
            &VertexBufferDesc {
                layout,
                vertex_count: (CAPACITY * 4) as u32,
                usage: BufferUsage::Dynamic,
            },
            None,
        )?;
        let indices: Vec<u8> = (0..CAPACITY as u16)
            .flat_map(|q| {
                let v = q * 4;
                [v, v + 1, v + 2, v, v + 2, v + 3]
            })
            .flat_map(u16::to_le_bytes)
            .collect();
        let ib = backend.create_index_buffer(
            &IndexBufferDesc {
                format: IndexFormat::U16,
                index_count: (CAPACITY * 6) as u32,
                usage: BufferUsage::Static,
            },
            Some(&indices),
        )?;
        let textures: usize = lib
            .effects
            .values()
            .flat_map(|e| &e.emitters)
            .map(|e| e.textures)
            .sum();
        let found: usize = lib
            .effects
            .values()
            .flat_map(|e| &e.emitters)
            .map(|e| e.found)
            .sum();
        eprintln!(
            "particles: {} effects, {} emitters, {found} of {textures} textures on the atlas, {} templates show effects",
            lib.effects.len(),
            lib.effects
                .values()
                .map(|e| e.emitters.len())
                .sum::<usize>(),
            lib.by_template.len()
        );
        Ok(Self {
            lib,
            sim: Particles::default(),
            vb,
            ib,
            blended,
            glowing,
            sprites: Vec::new(),
            draw_runs: 0,
            vertices: Vec::new(),
            ids: HashMap::new(),
            by_hash: None,
            resolved: HashMap::new(),
            flashes: Vec::new(),
            clock: 0.0,
            held_asks: HashMap::new(),
        })
    }

    /// Sets off what the fight asked for. Returns the effect templates a
    /// model's state started (`StartEmitter`, [`FxTemplate::Hash`]) that
    /// came into being as objects this time, with where: a held one only
    /// as it starts, not while its source keeps asking ([`fresh_hold`]).
    /// From the 360 code: `StartEmitter` (verb 0x3347914F, handler
    /// 0x8222FA30 calling 0x8222FA68, also behind
    /// `lua_ObjectState_StartEmitter`) finds the template by the effect's
    /// name hash (0x82408B88) and spawns an object of it through the
    /// game's spawner (0x82409B38, as `lua_Pg_SpawnRelative` does), so
    /// the template's components come into being with it, its
    /// `SoundEffect` among them ([`m2_engine::spawn_sound`]).
    pub fn take(&mut self, asked: Vec<GameFx>, templates: &Templates) -> Vec<(u32, Vec3)> {
        let mut created = Vec::new();
        for fx in asked {
            let (label, id) = match &fx.template {
                FxTemplate::Id(id) => (format!("{id:08x}"), Some(*id)),
                FxTemplate::Name(n) => {
                    let id = *self.ids.entry(n.clone()).or_insert_with(|| templates.id(n));
                    (n.clone(), id)
                }
                FxTemplate::Hash(h) => {
                    let by_hash = self.by_hash.get_or_insert_with(|| {
                        let mut ids: Vec<(&u32, &String)> = templates.names.iter().collect();
                        // Of twins by name, the lowest id's.
                        ids.sort_unstable_by_key(|&(&id, _)| std::cmp::Reverse(id));
                        ids.into_iter()
                            .map(|(&id, n)| (m2_assets::name_hash(n), id))
                            .collect()
                    });
                    let id = by_hash.get(h).copied();
                    let label = id
                        .and_then(|id| templates.names.get(&id).cloned())
                        .unwrap_or_else(|| format!("#{h:08x}"));
                    (label, id)
                }
            };
            if let (FxTemplate::Hash(_), Some(id)) = (&fx.template, id) {
                let fresh = match fx.hold {
                    Some(key) => fresh_hold(&mut self.held_asks, key, self.clock),
                    None => true,
                };
                if fresh {
                    if !self.resolved.contains_key(&label) {
                        eprintln!("state effect: {label} ({id:08x}) comes into being");
                    }
                    created.push((id, fx.at));
                }
            }
            let effects: Vec<u32> =
                id.map_or_else(Vec::new, |id| self.lib.of_template(id).to_vec());
            let first = !self.resolved.contains_key(&label);
            let r = self.resolved.entry(label.clone()).or_default();
            r.asked += 1;
            if first {
                let specs: Vec<_> = effects
                    .iter()
                    .filter_map(|h| self.lib.effects.get(h))
                    .collect();
                r.effects = specs.len();
                r.emitters = specs.iter().map(|s| s.emitters.len()).sum();
                r.textured = specs
                    .iter()
                    .flat_map(|s| &s.emitters)
                    .filter(|e| e.textures > 0 && e.found == e.textures)
                    .count();
                r.glowing = specs
                    .iter()
                    .flat_map(|s| &s.emitters)
                    .filter(|e| e.glows())
                    .count();
                eprintln!(
                    "particles: {label} -> {} effects {:08x?}, {} emitters, {} textured, {} glowing",
                    r.effects, effects, r.emitters, r.textured, r.glowing
                );
            }
            for (k, h) in effects.into_iter().enumerate() {
                // Each as long as its template says (`RedEffectComponent`).
                let run = id.and_then(|id| self.lib.run(id, k));
                match (fx.hold, run) {
                    (Some(key), _) => self.sim.hold(&self.lib, key, h, fx.at, 1.0),
                    (None, Some(run)) => self.sim.spawn_run(&self.lib, h, fx.at, run, 1.0),
                    (None, None) => self.sim.spawn(&self.lib, h, fx.at, 0.0, 1.0),
                };
            }
            let Some(id) = id else { continue };
            // What it sets off with it ([`m2_engine::lights::links`]): each
            // child's effects and light at its offset, and its own
            // children in turn (an explosion's flash and dirt effects, its
            // frag explosion, its light 2 m up).
            let tree = link_tree(id, &templates.links);
            if first && tree.len() > 1 {
                let named: Vec<String> = tree[1..]
                    .iter()
                    .map(|(c, at)| {
                        let n = templates
                            .names
                            .get(c)
                            .cloned()
                            .unwrap_or(format!("{c:08x}"));
                        format!("{n} at {:.1?}", at.to_array())
                    })
                    .collect();
                eprintln!("links: {label} sets off {}", named.join(", "));
            }
            for (k, &(child, offset)) in tree.iter().enumerate() {
                let at = fx.at + offset;
                if k > 0 {
                    for &h in self.lib.of_template(child) {
                        match fx.hold {
                            Some(key) => self.sim.hold(&self.lib, key, h, at, 1.0),
                            None => self.sim.spawn(&self.lib, h, at, 0.0, 1.0),
                        };
                    }
                }
                // A light: a flash that runs its envelope, or a held fire's
                // glow for as long as its source keeps calling.
                let Some(spec) = templates.lights.get(&child) else {
                    continue;
                };
                if first {
                    eprintln!(
                        "lights: {label} -> {:?} within {} m, gone at {} m, {:.1?} off, {}",
                        spec.color,
                        spec.inner,
                        spec.outer,
                        offset.to_array(),
                        match spec.envelope {
                            Some([a, b, c]) => format!("up {a} s, held {b} s, down {c} s"),
                            None => "steady".into(),
                        }
                    );
                }
                let key = fx.hold.map(|k| k ^ u64::from(child));
                match key.and_then(|key| self.flashes.iter_mut().find(|f| f.key == Some(key))) {
                    Some(f) => {
                        f.at = at;
                        f.since_call = 0.0;
                    }
                    None => self.flashes.push(Flash {
                        at,
                        spec: spec.clone(),
                        age: 0.0,
                        key,
                        since_call: 0.0,
                    }),
                }
            }
        }
        created
    }

    /// The camera the effects kept about it follow.
    pub fn set_camera(&mut self, eye: Vec3, right: Vec3, up: Vec3, forward: Vec3) {
        self.sim.set_camera(eye, right, up, forward);
    }

    /// Swaps the ambient effects for a record's (`AmbientEffect0..4`): the
    /// last ones deleted with their particles, each new one with a
    /// template of type 0 started about the camera, as the game does on
    /// every change of the light (from the 360 code, 0x8224D650). Type 1
    /// (on the ground about the camera) is left out: no record uses it.
    /// Returns how many effects started.
    pub fn set_ambient(
        &mut self,
        effects: &[m2_engine::atmosphere::AmbientEffect],
        templates: &Templates,
    ) -> usize {
        self.sim.clear_about_camera();
        let mut started = 0;
        for a in effects.iter().filter(|a| a.template != 0 && a.kind == 0) {
            let Some(id) = self.template_by_hash(a.template, templates) else {
                eprintln!("particles: ambient effect #{:08x}: no template", a.template);
                continue;
            };
            for fx in self.lib.of_template(id).to_vec() {
                if self
                    .sim
                    .spawn_about_camera(&self.lib, fx, a.radius, a.height)
                {
                    started += 1;
                }
            }
        }
        started
    }

    /// The template whose name hashes to `h` (of twins, the lowest id).
    fn template_by_hash(&mut self, h: u32, templates: &Templates) -> Option<u32> {
        let by_hash = self.by_hash.get_or_insert_with(|| {
            let mut ids: Vec<(&u32, &String)> = templates.names.iter().collect();
            ids.sort_unstable_by_key(|&(&id, _)| std::cmp::Reverse(id));
            ids.into_iter()
                .map(|(&id, n)| (m2_assets::name_hash(n), id))
                .collect()
        });
        by_hash.get(&h).copied()
    }

    pub fn step(&mut self, dt: f32) {
        self.sim.step(&self.lib, dt);
        self.clock += dt;
        let now = self.clock;
        self.held_asks.retain(|_, at| now - *at <= HOLD);
        for f in &mut self.flashes {
            f.age += dt;
            f.since_call += dt;
        }
        // A held one burns while its source calls; a flash runs its
        // envelope; a steady light set off once (a rocket's contrail light,
        // set off as it flies) goes out as a held one does when its source
        // stops (inferred: the data gives it no time).
        self.flashes.retain(|f| match (f.key, f.spec.ends()) {
            (None, Some(end)) => f.age < end,
            _ => f.since_call < m2_engine::particles::HOLD,
        });
    }

    /// The lights burning now, the nearest `eye` first, as many as a
    /// frame draws ([`m2_render::POINT_LIGHTS`]).
    pub fn point_lights(&self, eye: Vec3) -> [PointLight; m2_render::POINT_LIGHTS] {
        let mut lit: Vec<PointLight> = self
            .flashes
            .iter()
            .filter_map(|f| {
                let level = f.spec.level(f.age);
                (level > 0.0).then(|| PointLight {
                    position: f.at,
                    color: f.spec.color * level,
                    inner: f.spec.inner,
                    outer: f.spec.outer,
                })
            })
            .collect();
        lit.sort_by(|a, b| {
            a.position
                .distance_squared(eye)
                .total_cmp(&b.position.distance_squared(eye))
        });
        let mut out = [PointLight::default(); m2_render::POINT_LIGHTS];
        for (slot, light) in out.iter_mut().zip(lit) {
            *slot = light;
        }
        out
    }

    /// How many lights burn now.
    pub fn lights_lit(&self) -> usize {
        self.flashes
            .iter()
            .filter(|f| f.spec.level(f.age) > 0.0)
            .count()
    }

    /// Particles alive.
    pub fn live(&self) -> usize {
        self.sim.live()
    }

    /// How far the particles sit from `centre` on average, metres
    /// ([`Particles::mean_distance`]).
    pub fn mean_distance(&self, centre: Vec3) -> f32 {
        self.sim.mean_distance(centre)
    }

    /// Queues every particle: writes this frame's squares and draws them
    /// in two batches.
    pub fn draw(
        &mut self,
        backend: &mut dyn RenderBackend,
        queue: &mut DrawQueue,
        view: &View,
    ) -> anyhow::Result<()> {
        self.sprites.clear();
        self.sim.sprites(&self.lib, &mut self.sprites);
        if self.sprites.is_empty() {
            return Ok(());
        }
        let eye = view.eye();
        // Back to front, whatever each one's blend: a fireball's smoke is
        // drawn behind its glow where it is behind and over it where it
        // is in front. The two blends then alternate in runs, each a draw.
        self.sprites.sort_by(|a, b| {
            b.at.distance_squared(eye)
                .total_cmp(&a.at.distance_squared(eye))
        });
        self.sprites.truncate(CAPACITY);
        let v = view.view;
        let right = Vec3::new(v.x_axis.x, v.y_axis.x, v.z_axis.x);
        let up = Vec3::new(v.x_axis.y, v.y_axis.y, v.z_axis.y);
        self.vertices.clear();
        self.vertices.reserve(self.sprites.len() * 4 * VERTEX);
        // Smoke and dust take the effects' own lights (a blast's flash
        // lights its smoke white and orange, as Brendan's footage of a
        // satellite bomb shows), as a surface facing every way would:
        // each light's colour times its reach, no N.L (inferred).
        let lights = self.point_lights(eye);
        for s in &self.sprites {
            let light = if s.glow {
                Vec3::ZERO
            } else {
                sprite_light(&lights, s.at)
            };
            quad(&mut self.vertices, s, right, up, light);
        }
        let bytes: Vec<u8> = self.vertices.iter().flat_map(|f| f.to_le_bytes()).collect();
        backend.write_vertex_buffer(self.vb, 0, &bytes)?;
        // Each run of one blend is a draw, sorted by its nearest sprite:
        // the draw queue orders translucent draws by their sort origins'
        // depth alone, and every sprite of the next run is nearer than
        // this run's nearest, so the runs keep their order through it.
        let run_list = runs(&self.sprites);
        self.draw_runs = run_list.len();
        for (first, end, glow) in run_list {
            queue.push(Primitive {
                call: DrawCall {
                    material: if glow { self.glowing } else { self.blended },
                    topology: Topology::TriangleList,
                    vertex_buffer: self.vb,
                    geometry: Geometry::Indexed {
                        buffer: self.ib,
                        first: (first * 6) as u32,
                        count: ((end - first) * 6) as u32,
                        base_vertex: 0,
                    },
                    world: Mat4::IDENTITY,
                    skin: None,
                    instances: None,
                    uv_offset: [0.0; 2],
                    tint: [1.0; 4],
                    beams_lit: false,
                },
                layer: Layer::WORLD,
                translucent: true,
                sort_origin: self.sprites[end - 1].at,
                user: 0,
            });
        }
        Ok(())
    }
}

/// How deep a template's links are followed (an explosion, its frag
/// explosion, that one's effects). Invented: no link chain in the data
/// goes deeper than three on the explosions looked at.
const LINK_DEPTH: usize = 3;

/// Template `id` and everything its links set off, each with its offset
/// from `id` (offsets add down the chain), `id` first, each template once.
fn link_tree(id: u32, links: &HashMap<u32, Vec<m2_engine::lights::Link>>) -> Vec<(u32, Vec3)> {
    let mut out = vec![(id, Vec3::ZERO)];
    let mut level = vec![(id, Vec3::ZERO)];
    for _ in 0..LINK_DEPTH {
        let mut next = Vec::new();
        for (t, at) in level {
            for l in links.get(&t).map_or(&[][..], Vec::as_slice) {
                if out.iter().all(|&(c, _)| c != l.child) {
                    out.push((l.child, at + l.offset));
                    next.push((l.child, at + l.offset));
                }
            }
        }
        level = next;
    }
    out
}

/// The runs of one blend in `sprites` (sorted back to front): `(first,
/// end, glow)` of each, in order; each is one draw.
fn runs(sprites: &[Sprite]) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    let mut first = 0;
    while first < sprites.len() {
        let glow = sprites[first].glow;
        let mut end = first + 1;
        while end < sprites.len() && sprites[end].glow == glow {
            end += 1;
        }
        out.push((first, end, glow));
        first = end;
    }
    out
}

/// The light `lights` throw on a point: each one's colour times its
/// reach there (1 within `inner`, 0 at `outer`).
fn sprite_light(lights: &[PointLight], at: Vec3) -> Vec3 {
    lights
        .iter()
        .filter(|l| l.outer > 0.0)
        .map(|l| {
            let d = l.position.distance(at);
            let reach = ((l.outer - d) / (l.outer - l.inner).max(1e-3)).clamp(0.0, 1.0);
            l.color * reach
        })
        .sum()
}

/// Sprite `s` as four vertices facing the camera (`right`, `up`).
fn quad(out: &mut Vec<f32>, s: &Sprite, right: Vec3, up: Vec3, light: Vec3) {
    let (sin, cos) = s.turn.sin_cos();
    // The corners at plus or minus half the width, as the game's particle
    // vertex shaders put them (the PC's 346; the 360's PgFXVP).
    let half = s.size * 0.5;
    let r = (right * cos + up * sin) * half;
    let u = (up * cos - right * sin) * half;
    let [l, t, w, h] = s.uv;
    let shade = if s.glow { 1.0 } else { SHADE };
    let [cr, cg, cb, ca] = s.colour;
    let colour = [
        cr * (shade + light.x),
        cg * (shade + light.y),
        cb * (shade + light.z),
        ca,
    ];
    for (corner, uv) in [
        (-r + u, [l, t]),
        (r + u, [l + w, t]),
        (r - u, [l + w, t + h]),
        (-r - u, [l, t + h]),
    ] {
        let p = s.at + corner;
        out.extend([p.x, p.y, p.z, uv[0], uv[1]]);
        out.extend(colour);
    }
}

/// Whether the held effect `key`, asked for at `now` (seconds), comes
/// into being afresh: when it was not asked for in the last [`HOLD`]
/// seconds, the time a held effect outlives its last ask. Records the
/// ask. Inferred: a held effect stands for the 360's emitter object
/// living on while its state lasts, so it is created, and heard, once.
fn fresh_hold(asks: &mut HashMap<u64, f32>, key: u64, now: f32) -> bool {
    let fresh = asks.get(&key).is_none_or(|&at| now - at > HOLD);
    asks.insert(key, now);
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_effect_comes_into_being_once_while_asked_for() {
        let mut asks = HashMap::new();
        assert!(fresh_hold(&mut asks, 7, 0.0));
        assert!(!fresh_hold(&mut asks, 7, 0.3));
        assert!(!fresh_hold(&mut asks, 7, 0.6));
        // Another source's is its own.
        assert!(fresh_hold(&mut asks, 8, 0.6));
        // Asked for again after it went out: anew.
        assert!(fresh_hold(&mut asks, 7, 0.6 + HOLD + 0.1));
    }

    #[test]
    fn a_quad_faces_the_camera_and_covers_its_atlas_cell() {
        let s = Sprite {
            at: Vec3::new(1.0, 2.0, 3.0),
            size: 2.0,
            turn: 0.0,
            uv: [0.25, 0.5, 0.125, 0.25],
            colour: [2.0, 1.0, 0.5, 0.75],
            glow: true,
        };
        let mut v = Vec::new();
        quad(&mut v, &s, Vec3::X, Vec3::Y, Vec3::ZERO);
        assert_eq!(v.len(), 4 * VERTEX);
        // Top left, then clockwise; 1 m each way from the middle of a 2 m
        // sprite.
        assert_eq!(&v[0..5], &[0.0, 3.0, 3.0, 0.25, 0.5]);
        assert_eq!(
            &v[2 * VERTEX..2 * VERTEX + 5],
            &[2.0, 1.0, 3.0, 0.375, 0.75]
        );
        // A glow keeps its colour; a blended one is shaded.
        assert_eq!(&v[5..9], &[2.0, 1.0, 0.5, 0.75]);
        let mut b = Vec::new();
        quad(
            &mut b,
            &Sprite { glow: false, ..s },
            Vec3::X,
            Vec3::Y,
            Vec3::ZERO,
        );
        assert_eq!(b[5], 2.0 * SHADE);
    }

    /// Sorted back to front, a glow between two smokes makes three runs;
    /// neighbours of one blend share a run.
    /// `Explosion (Grenade)` (1) sets off its flash (2), its light 2 m up
    /// (3) and its frag explosion (4), which links back to the flash and
    /// on to its own debris 1 m along x (5): each once, offsets added.
    #[test]
    fn a_templates_links_set_off_a_tree_each_once() {
        use m2_engine::lights::Link;
        let link = |child, offset| Link { child, offset };
        let links = HashMap::from([
            (
                1,
                vec![
                    link(2, Vec3::ZERO),
                    link(3, Vec3::Y * 2.0),
                    link(4, Vec3::ZERO),
                ],
            ),
            (4, vec![link(2, Vec3::ZERO), link(5, Vec3::X)]),
            (5, vec![link(1, Vec3::ZERO)]),
        ]);
        assert_eq!(
            link_tree(1, &links),
            vec![
                (1, Vec3::ZERO),
                (2, Vec3::ZERO),
                (3, Vec3::Y * 2.0),
                (4, Vec3::ZERO),
                (5, Vec3::X),
            ]
        );
        assert_eq!(link_tree(9, &links), vec![(9, Vec3::ZERO)]);
    }

    /// A 12-times-white flash 6 m across lights smoke beside it fully,
    /// half-way out by half, and none past its reach; an unlit slot adds
    /// nothing.
    #[test]
    fn smoke_takes_the_flash_by_its_reach() {
        let flash = PointLight {
            position: Vec3::new(0.0, 2.0, 0.0),
            color: Vec3::splat(12.0),
            inner: 0.0,
            outer: 6.0,
        };
        let lights = [flash, PointLight::default()];
        assert_eq!(sprite_light(&lights, flash.position), Vec3::splat(12.0));
        assert!((sprite_light(&lights, Vec3::new(3.0, 2.0, 0.0)).x - 6.0).abs() < 1e-4);
        assert_eq!(sprite_light(&lights, Vec3::new(0.0, 9.0, 0.0)), Vec3::ZERO);
    }

    #[test]
    fn runs_split_the_sorted_sprites_by_blend() {
        let s = |glow: bool| Sprite {
            at: Vec3::ZERO,
            size: 1.0,
            turn: 0.0,
            uv: [0.0; 4],
            colour: [1.0; 4],
            glow,
        };
        let sprites = [s(false), s(false), s(true), s(false), s(true), s(true)];
        assert_eq!(
            runs(&sprites),
            vec![(0, 2, false), (2, 3, true), (3, 4, false), (4, 6, true)]
        );
        assert!(runs(&[]).is_empty());
    }
}
