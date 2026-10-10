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

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

use anyhow::Context as _;
use m2_assets::effect::ATLAS_TEXTURE;
use m2_engine::AssetSource;
use m2_engine::model::{MaterialCache, upload_texture};
use m2_engine::particles::{Library, Particles, Sprite};
use m2_engine::world::Templates;
use m2_math::glam::{Mat4, Vec3};
use m2_render::{
    BlendMode, BufferUsage, DrawCall, DrawQueue, Geometry, IndexBufferDesc, IndexBufferHandle,
    IndexFormat, Layer, MaterialDesc, MaterialId, Primitive, RenderBackend, Topology,
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
    vertices: Vec<f32>,
    /// Templates asked for by name, resolved to ids.
    ids: HashMap<String, Option<u32>>,
    /// Per template asked for (its name, or its id in hex).
    pub resolved: HashMap<String, Resolved>,
}

impl ParticleDraw {
    /// Reads every effect and the atlas, and uploads the atlas.
    pub fn load(
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        source: &mut AssetSource<BufReader<File>>,
        templates: &Templates,
    ) -> anyhow::Result<Self> {
        let lib = Library::load(source, templates.effects.clone()).context("effects")?;
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
            vertices: Vec::new(),
            ids: HashMap::new(),
            resolved: HashMap::new(),
        })
    }

    /// Sets off what the fight asked for.
    pub fn take(&mut self, asked: Vec<GameFx>, templates: &Templates) {
        for fx in asked {
            let (label, id) = match &fx.template {
                FxTemplate::Id(id) => (format!("{id:08x}"), Some(*id)),
                FxTemplate::Name(n) => {
                    let id = *self.ids.entry(n.clone()).or_insert_with(|| templates.id(n));
                    (n.clone(), id)
                }
            };
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
                    "particles: {label} -> {} effects, {} emitters, {} textured, {} glowing",
                    r.effects, r.emitters, r.textured, r.glowing
                );
            }
            for h in effects {
                match fx.hold {
                    Some(key) => self.sim.hold(&self.lib, key, h, fx.at, 1.0),
                    None => self.sim.spawn(&self.lib, h, fx.at, 0.0, 1.0),
                };
            }
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.sim.step(&self.lib, dt);
    }

    /// Particles alive.
    pub fn live(&self) -> usize {
        self.sim.live()
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
        // Blended ones first, back to front; then the added ones.
        self.sprites.sort_by(|a, b| {
            a.glow.cmp(&b.glow).then_with(|| {
                b.at.distance_squared(eye)
                    .total_cmp(&a.at.distance_squared(eye))
            })
        });
        self.sprites.truncate(CAPACITY);
        let v = view.view;
        let right = Vec3::new(v.x_axis.x, v.y_axis.x, v.z_axis.x);
        let up = Vec3::new(v.x_axis.y, v.y_axis.y, v.z_axis.y);
        self.vertices.clear();
        self.vertices.reserve(self.sprites.len() * 4 * VERTEX);
        for s in &self.sprites {
            quad(&mut self.vertices, s, right, up);
        }
        let bytes: Vec<u8> = self.vertices.iter().flat_map(|f| f.to_le_bytes()).collect();
        backend.write_vertex_buffer(self.vb, 0, &bytes)?;
        let blended = self.sprites.iter().filter(|s| !s.glow).count();
        let centre =
            |range: &[Sprite]| range.iter().map(|s| s.at).sum::<Vec3>() / range.len().max(1) as f32;
        let batches = [
            (self.blended, 0, blended),
            (self.glowing, blended, self.sprites.len() - blended),
        ];
        for (material, first, n) in batches {
            if n == 0 {
                continue;
            }
            queue.push(Primitive {
                call: DrawCall {
                    material,
                    topology: Topology::TriangleList,
                    vertex_buffer: self.vb,
                    geometry: Geometry::Indexed {
                        buffer: self.ib,
                        first: (first * 6) as u32,
                        count: (n * 6) as u32,
                        base_vertex: 0,
                    },
                    world: Mat4::IDENTITY,
                    skin: None,
                    instances: None,
                    uv_offset: [0.0; 2],
                },
                layer: Layer::WORLD,
                translucent: true,
                sort_origin: centre(&self.sprites[first..first + n]),
                user: 0,
            });
        }
        Ok(())
    }
}

/// Sprite `s` as four vertices facing the camera (`right`, `up`).
fn quad(out: &mut Vec<f32>, s: &Sprite, right: Vec3, up: Vec3) {
    let (sin, cos) = s.turn.sin_cos();
    let half = s.size;
    let r = (right * cos + up * sin) * half;
    let u = (up * cos - right * sin) * half;
    let [l, t, w, h] = s.uv;
    let shade = if s.glow { 1.0 } else { SHADE };
    let [cr, cg, cb, ca] = s.colour;
    let colour = [cr * shade, cg * shade, cb * shade, ca];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_faces_the_camera_and_covers_its_atlas_cell() {
        let s = Sprite {
            at: Vec3::new(1.0, 2.0, 3.0),
            size: 1.0,
            turn: 0.0,
            uv: [0.25, 0.5, 0.125, 0.25],
            colour: [2.0, 1.0, 0.5, 0.75],
            glow: true,
        };
        let mut v = Vec::new();
        quad(&mut v, &s, Vec3::X, Vec3::Y);
        assert_eq!(v.len(), 4 * VERTEX);
        // Top left, then clockwise; 1 m each way from the middle.
        assert_eq!(&v[0..5], &[0.0, 3.0, 3.0, 0.25, 0.5]);
        assert_eq!(
            &v[2 * VERTEX..2 * VERTEX + 5],
            &[2.0, 1.0, 3.0, 0.375, 0.75]
        );
        // A glow keeps its colour; a blended one is shaded.
        assert_eq!(&v[5..9], &[2.0, 1.0, 0.5, 0.75]);
        let mut b = Vec::new();
        quad(&mut b, &Sprite { glow: false, ..s }, Vec3::X, Vec3::Y);
        assert_eq!(b[5], 2.0 * SHADE);
    }
}
